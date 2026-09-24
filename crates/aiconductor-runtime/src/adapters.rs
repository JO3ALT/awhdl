use crate::capability::{CapabilityRequest, file_resource};
use crate::config::{ActionConfig, McpAdapterKind as AdapterKind, ProjectConfig};
use crate::mcp::summarize_content;
use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::fs;

impl AdapterKind {
    pub fn resolve(route: &ActionConfig, tool: &str) -> Result<Self> {
        if let Some(adapter) = route.adapter {
            return Ok(adapter);
        }
        match tool {
            "codex" => Ok(Self::Codex),
            "lean" | "check_lean_code" | "check_lean_file" | "get_lean_environment" => {
                Ok(Self::Lean)
            }
            "prolog" | "run_prolog" | "run_prolog_file" => Ok(Self::Prolog),
            "matlab" | "evaluate_matlab_code" | "run_matlab_file" | "run_matlab_test_file" => {
                Ok(Self::Matlab)
            }
            "passthrough" => Ok(Self::Passthrough),
            other => bail!("unknown MCP adapter: {other}"),
        }
    }
}

pub struct AdapterInput<'a> {
    pub action: &'a str,
    pub route: &'a ActionConfig,
    pub requested_tool: Option<&'a str>,
    pub input: &'a str,
    pub source_instruction: &'a str,
    pub arguments: Value,
}

#[derive(Debug, Clone)]
pub struct PreparedMcpCall {
    pub adapter: AdapterKind,
    pub tool: String,
    pub arguments: Value,
}

#[derive(Debug, Serialize)]
pub struct NormalizedMcpOutput {
    pub adapter: AdapterKind,
    pub tool: String,
    pub text: String,
    pub structured: Option<Value>,
    pub raw_bytes: usize,
    pub truncated: bool,
}

impl NormalizedMcpOutput {
    pub fn for_planner(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }
}

pub fn prepare_call(config: &ProjectConfig, source: AdapterInput<'_>) -> Result<PreparedMcpCall> {
    let mut tool = resolve_tool(source.route, source.requested_tool, &source)?;
    let adapter = AdapterKind::resolve(source.route, &tool)?;
    let mut arguments = source.arguments.as_object().cloned().unwrap_or_default();

    let rewritten = match adapter {
        AdapterKind::Codex => {
            prepare_codex(config, &source, &mut arguments)?;
            None
        }
        AdapterKind::Lean => prepare_lean(config, &tool, &source, &mut arguments)?,
        AdapterKind::Prolog => prepare_prolog(config, &tool, &source, &mut arguments)?,
        AdapterKind::Matlab => prepare_matlab(config, &tool, &source, &mut arguments)?,
        AdapterKind::Passthrough => None,
    };
    if let Some(rewritten) = rewritten {
        tool = rewritten;
    }

    Ok(PreparedMcpCall {
        adapter,
        tool,
        arguments: Value::Object(arguments),
    })
}

pub fn normalize_result(
    adapter: AdapterKind,
    tool: &str,
    result: &Value,
    max_bytes: usize,
) -> Result<NormalizedMcpOutput> {
    let raw = summarize_content(result);
    let raw_bytes = raw.len();
    let structured = extract_structured_result(adapter, &raw);
    if let Some(error) = structured
        .as_ref()
        .and_then(Value::as_object)
        .filter(|object| object.get("ok").and_then(Value::as_bool) == Some(false))
    {
        bail!(
            "typed MCP result reported failure: {}",
            error
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unspecified adapter error")
        );
    }
    let (text, truncated) = truncate_owned(raw, max_bytes);
    Ok(NormalizedMcpOutput {
        adapter,
        tool: tool.to_owned(),
        text,
        structured,
        raw_bytes,
        truncated,
    })
}

fn resolve_tool(
    route: &ActionConfig,
    requested: Option<&str>,
    source: &AdapterInput<'_>,
) -> Result<String> {
    if let Some(fixed) = &route.tool {
        if requested.is_some_and(|value| value != fixed) {
            bail!("requested MCP tool is not allowed for this action");
        }
        return Ok(fixed.clone());
    }
    if let Some(selected) = requested
        && route.preferred_tools.iter().any(|item| item == selected)
    {
        return Ok(selected.to_owned());
    }
    let source_text = format!("{}\n{}", source.input, source.source_instruction).to_lowercase();
    if let Some(explicit) = route
        .preferred_tools
        .iter()
        .filter(|tool| source_text.contains(tool.as_str()))
        .max_by_key(|tool| tool.len())
    {
        return Ok(explicit.clone());
    }
    let inferred = [
        ("run_prolog_file", [".pl", ".pro", ".prolog"].as_slice()),
        ("check_lean_file", [".lean"].as_slice()),
        ("run_matlab_file", [".m"].as_slice()),
    ]
    .into_iter()
    .find(|(tool, extensions)| {
        route.preferred_tools.iter().any(|item| item == tool)
            && extensions
                .iter()
                .any(|extension| source_text.contains(extension))
    })
    .map(|(tool, _)| tool.to_owned());
    if let Some(tool) = inferred {
        return Ok(tool);
    }
    route
        .preferred_tools
        .first()
        .cloned()
        .context("MCP action has no preferred tool")
}

/// Codex sandbox and network settings expressed as capability requests.
/// Codex cannot restrict outbound hosts, so network access is `host:*`; full
/// access always includes the network, whatever `network_access` says.
pub fn codex_capability_requests(route: &ActionConfig) -> Result<Vec<CapabilityRequest>> {
    let sandbox = route.sandbox.as_deref().unwrap_or("read-only");
    let action = match sandbox {
        "read-only" => "sandbox.read_only",
        "workspace-write" => "sandbox.workspace_write",
        "danger-full-access" => "sandbox.full_access",
        other => bail!("unknown Codex sandbox {other}"),
    };
    let mut requests = vec![CapabilityRequest::new(action, "host:local")];
    if sandbox == "danger-full-access" || (route.network_access && sandbox == "workspace-write") {
        requests.push(CapabilityRequest::new("network.connect", "host:*"));
    }
    Ok(requests)
}

fn prepare_codex(
    config: &ProjectConfig,
    source: &AdapterInput<'_>,
    object: &mut Map<String, Value>,
) -> Result<()> {
    for request in codex_capability_requests(source.route)? {
        config.authorize_route(source.action, &request)?;
    }
    object.insert(
        "prompt".to_owned(),
        Value::String(effective_input(source).to_owned()),
    );
    let cwd = source
        .route
        .workdir
        .as_deref()
        .map(|value| config.resolve_path(value))
        .unwrap_or_else(|| config.root.clone());
    fs::create_dir_all(&cwd)?;
    object.insert(
        "cwd".to_owned(),
        Value::String(cwd.to_string_lossy().into_owned()),
    );
    object.insert(
        "sandbox".to_owned(),
        Value::String(
            source
                .route
                .sandbox
                .as_deref()
                .unwrap_or("read-only")
                .to_owned(),
        ),
    );
    object.insert(
        "approval-policy".to_owned(),
        Value::String(
            source
                .route
                .approval_policy
                .as_deref()
                .unwrap_or("never")
                .to_owned(),
        ),
    );
    if source.route.network_access
        && source.route.sandbox.as_deref().unwrap_or("read-only") == "workspace-write"
    {
        object.insert(
            "config".to_owned(),
            json!({"sandbox_workspace_write.network_access": true}),
        );
    } else {
        object.remove("config");
    }
    Ok(())
}

fn effective_input<'a>(source: &'a AdapterInput<'_>) -> &'a str {
    if source.input.trim().is_empty() {
        source.source_instruction
    } else {
        source.input
    }
}

/// The Lean and Prolog servers run on a remote host that sees this workspace
/// only through delayed file synchronization, so project files are read here
/// (after the capability check) and sent as code. Returns a replacement tool.
fn prepare_lean(
    config: &ProjectConfig,
    tool: &str,
    source: &AdapterInput<'_>,
    object: &mut Map<String, Value>,
) -> Result<Option<String>> {
    match tool {
        "check_lean_code" => {
            if !object.contains_key("code") {
                let code = find_lean_code([source.source_instruction, source.input])
                    .unwrap_or_else(|| source.input.to_owned());
                required_text(object, "code", &code)?;
            }
        }
        "check_lean_file" => {
            let file = required_path(object, "path", source, &[".lean"])?;
            let code = read_project_source(config, source.action, &file, MAX_LEAN_SOURCE_BYTES)?;
            let timeout = object.remove("timeout_sec");
            object.clear();
            object.insert("code".to_owned(), Value::String(code));
            if let Some(timeout) = timeout {
                object.insert("timeout_sec".to_owned(), timeout);
            }
            return Ok(Some("check_lean_code".to_owned()));
        }
        "get_lean_environment" => {}
        _ => bail!("Lean adapter does not support tool {tool}"),
    }
    Ok(None)
}

fn find_lean_code<'a>(candidates: impl IntoIterator<Item = &'a str>) -> Option<String> {
    const DECLARATIONS: [&str; 7] = [
        "theorem ",
        "example ",
        "def ",
        "structure ",
        "inductive ",
        "import ",
        "#check ",
    ];

    for candidate in candidates {
        let mut fenced = candidate.split("```");
        let _before = fenced.next();
        if let Some(block) = fenced.next() {
            let block = block
                .strip_prefix("lean\n")
                .or_else(|| block.strip_prefix("Lean\n"))
                .unwrap_or(block)
                .trim();
            if !block.is_empty() {
                return Some(block.to_owned());
            }
        }

        if let Some(start) = DECLARATIONS
            .iter()
            .filter_map(|marker| candidate.find(marker))
            .min()
        {
            return Some(candidate[start..].trim().to_owned());
        }
    }
    None
}

fn prepare_prolog(
    config: &ProjectConfig,
    tool: &str,
    source: &AdapterInput<'_>,
    object: &mut Map<String, Value>,
) -> Result<Option<String>> {
    match tool {
        "run_prolog" => {
            required_text(object, "query", source.input)?;
        }
        "run_prolog_file" => {
            let file = required_path(object, "file", source, &[".pl", ".pro", ".prolog"])?;
            let program =
                read_project_source(config, source.action, &file, MAX_PROLOG_SOURCE_BYTES)?;
            if !object.contains_key("query")
                && let Some(query) =
                    find_code_span([source.input, source.source_instruction], |value| {
                        value.contains("(") && value.ends_with('.')
                    })
            {
                object.insert("query".to_owned(), Value::String(query));
            }
            object.remove("file");
            object
                .entry("query".to_owned())
                .or_insert_with(|| Value::String("true.".to_owned()));
            object.insert("program_text".to_owned(), Value::String(program));
            return Ok(Some("run_prolog".to_owned()));
        }
        _ => bail!("Prolog adapter does not support tool {tool}"),
    }
    Ok(None)
}

/// The MATLAB host cannot see this workspace, so a project `.m` file is read
/// here (after the capability check) and sent as code. Returns a replacement tool.
fn prepare_matlab(
    config: &ProjectConfig,
    tool: &str,
    source: &AdapterInput<'_>,
    object: &mut Map<String, Value>,
) -> Result<Option<String>> {
    match tool {
        "evaluate_matlab_code" => {
            if !object.contains_key("code") {
                let code = find_matlab_run([source.input, source.source_instruction])
                    .unwrap_or_else(|| source.input.to_owned());
                required_text(object, "code", &code)?;
            }
            Ok(None)
        }
        "run_matlab_file" => {
            let file = required_path(object, "script_path", source, &[".m"])?;
            let code = read_project_source(config, source.action, &file, MAX_MATLAB_SCRIPT_BYTES)?;
            object.clear();
            object.insert("code".to_owned(), Value::String(code));
            Ok(Some("evaluate_matlab_code".to_owned()))
        }
        "run_matlab_test_file" => bail!(
            "run_matlab_test_file is unsupported: the MATLAB host cannot see workspace files and a test class cannot be sent as code"
        ),
        _ => bail!("MATLAB adapter does not support tool {tool}"),
    }
}

const MAX_MATLAB_SCRIPT_BYTES: u64 = 256 * 1024;
// Server-side limits: Lean MAX_CODE_BYTES, Prolog MAX_PROGRAM_BYTES.
const MAX_LEAN_SOURCE_BYTES: u64 = 512 * 1024;
const MAX_PROLOG_SOURCE_BYTES: u64 = 2 * 1024 * 1024;

/// Capability-check a project source file and return its UTF-8 contents.
fn read_project_source(
    config: &ProjectConfig,
    subject: &str,
    file: &str,
    max_bytes: u64,
) -> Result<String> {
    ensure_readable_project_file(config, subject, file)?;
    let path = config.resolve_path(file);
    let size = fs::metadata(&path)?.len();
    if size > max_bytes {
        bail!(
            "source file is larger than {max_bytes} bytes: {}",
            path.display()
        );
    }
    fs::read_to_string(&path)
        .with_context(|| format!("source file is not UTF-8: {}", path.display()))
}

fn required_text(object: &mut Map<String, Value>, key: &str, fallback: &str) -> Result<()> {
    object
        .entry(key.to_owned())
        .or_insert_with(|| Value::String(fallback.to_owned()));
    if object
        .get(key)
        .and_then(Value::as_str)
        .is_none_or(|value| value.trim().is_empty())
    {
        bail!("adapter could not produce required string argument: {key}");
    }
    Ok(())
}

fn required_path(
    object: &mut Map<String, Value>,
    key: &str,
    source: &AdapterInput<'_>,
    extensions: &[&str],
) -> Result<String> {
    if !object.contains_key(key)
        && let Some(path) = find_project_path([source.input, source.source_instruction], extensions)
    {
        object.insert(key.to_owned(), Value::String(path));
    }
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .with_context(|| format!("adapter could not produce required path argument: {key}"))
}

/// Planner-supplied paths pass the same capability check as every other access.
fn ensure_readable_project_file(config: &ProjectConfig, subject: &str, value: &str) -> Result<()> {
    let path = config.resolve_path(value);
    if !path.is_file() {
        bail!("adapter input file does not exist: {}", path.display());
    }
    let resource = file_resource(&config.root, value)?;
    config.authorize_route(subject, &CapabilityRequest::new("file.read", resource))?;
    Ok(())
}

fn find_project_path<const N: usize>(inputs: [&str; N], extensions: &[&str]) -> Option<String> {
    inputs.into_iter().find_map(|input| {
        let normalized = input.replace('\\', "/");
        let start = ["examples/", ".runtime/"]
            .iter()
            .find_map(|prefix| normalized.find(prefix))?;
        let tail = &normalized[start..];
        let end = extensions
            .iter()
            .filter_map(|extension| tail.find(extension).map(|index| index + extension.len()))
            .min()?;
        Some(tail[..end].to_owned())
    })
}

fn find_code_span<const N: usize>(
    inputs: [&str; N],
    predicate: impl Fn(&str) -> bool,
) -> Option<String> {
    inputs.into_iter().find_map(|input| {
        let mut remainder = input;
        while let Some(start) = remainder.find('`') {
            remainder = &remainder[start + 1..];
            let end = remainder.find('`')?;
            let span = remainder[..end].trim();
            if predicate(span) {
                return Some(span.to_owned());
            }
            remainder = &remainder[end + 1..];
        }
        None
    })
}

fn find_matlab_run<const N: usize>(inputs: [&str; N]) -> Option<String> {
    inputs.into_iter().find_map(|input| {
        let start = input.find("run(")?;
        let tail = &input[start..];
        let end = tail.find("));")? + 3;
        Some(tail[..end].replace("\\'", "'").replace("''", "','"))
    })
}

fn extract_structured_result(adapter: AdapterKind, text: &str) -> Option<Value> {
    if adapter == AdapterKind::Matlab
        && let Some(value) = text
            .lines()
            .find_map(|line| line.strip_prefix("AICONDUCTOR_STATS_JSON="))
            .and_then(|value| serde_json::from_str(value).ok())
    {
        return Some(value);
    }
    serde_json::from_str(text.trim()).ok()
}

fn truncate_owned(mut value: String, max: usize) -> (String, bool) {
    if value.len() <= max {
        return (value, false);
    }
    let mut end = max;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    value.push_str("\n[adapter output truncated]");
    (value, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProjectConfig;

    #[test]
    fn prolog_adapter_recovers_typed_file_and_query() {
        let root = crate::test_support::project_root();
        let config = ProjectConfig::load(root).unwrap();
        let route = config.action("logic_rules").unwrap();
        let call = prepare_call(
            &config,
            AdapterInput {
                action: "logic_rules",
                route,
                requested_tool: Some("run_prolog_file"),
                input: "Prologを実行する",
                source_instruction: "file `examples/open_data_population_pipeline/population_rules.pl`, query `population_assessment_from_data(Trend, Attention).`",
                arguments: json!({}),
            },
        )
        .unwrap();
        assert_eq!(call.adapter, AdapterKind::Prolog);
        // The file is sent as program text, since the Prolog host may not see it yet.
        assert_eq!(call.tool, "run_prolog");
        assert!(call.arguments.get("file").is_none());
        let program = std::fs::read_to_string(
            config.resolve_path("examples/open_data_population_pipeline/population_rules.pl"),
        )
        .unwrap();
        assert_eq!(
            call.arguments.get("program_text"),
            Some(&Value::String(program))
        );
        assert_eq!(
            call.arguments.get("query"),
            Some(&Value::String(
                "population_assessment_from_data(Trend, Attention).".to_owned()
            ))
        );
    }

    #[test]
    fn lean_adapter_recovers_source_from_operator_instruction() {
        let root = crate::test_support::project_root();
        let config = ProjectConfig::load(root).unwrap();
        let route = config.action("logic_proof").unwrap();
        let call = prepare_call(
            &config,
            AdapterInput {
                action: "logic_proof",
                route,
                requested_tool: Some("check_lean_code"),
                input: "Check the provided Lean theorem.",
                source_instruction: "次のLeanコードを検証: theorem identity (n : Nat) : n = n := by rfl",
                arguments: json!({}),
            },
        )
        .unwrap();
        assert_eq!(
            call.arguments.get("code"),
            Some(&Value::String(
                "theorem identity (n : Nat) : n = n := by rfl".to_owned()
            ))
        );
    }

    #[test]
    fn planner_paths_outside_granted_scopes_are_denied() {
        let root = crate::test_support::project_root();
        let config = ProjectConfig::load(&root).unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("rules.pl");
        std::fs::write(&outside_file, "fact.").unwrap();
        let prolog = |subject: &str, file: &str| {
            prepare_call(
                &config,
                AdapterInput {
                    action: subject,
                    route: config.action(subject).unwrap(),
                    requested_tool: Some("run_prolog_file"),
                    input: "run",
                    source_instruction: "run",
                    arguments: json!({"file": file, "query": "fact."}),
                },
            )
        };
        // Outside the project, inside it but ungranted, and a traversal back out.
        let denied = prolog("logic_rules", &outside_file.to_string_lossy()).unwrap_err();
        assert!(denied.to_string().contains("outside the project"));
        assert!(prolog("logic_rules", "config/runtime.toml").is_err());
        assert!(prolog("logic_rules", "examples/../config/runtime.toml").is_err());
        let granted = std::fs::read_dir(root.join("examples"))
            .unwrap()
            .flatten()
            .flat_map(|dir| {
                std::fs::read_dir(dir.path())
                    .into_iter()
                    .flatten()
                    .flatten()
            })
            .map(|entry| entry.path())
            .find(|path| path.extension().is_some_and(|ext| ext == "pl"))
            .expect("an example Prolog file");
        let relative = granted
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        assert!(prolog("logic_rules", &relative).is_ok());
    }

    #[test]
    fn lean_file_adapter_sends_the_verified_file_as_code() {
        let root = crate::test_support::project_root();
        let config = ProjectConfig::load(root).unwrap();
        let route = config.action("logic_proof").unwrap();
        let call = prepare_call(
            &config,
            AdapterInput {
                action: "logic_proof",
                route,
                requested_tool: Some("check_lean_file"),
                input: "examples/lean_mcp_smoke_test/valid.leanを検証する",
                source_instruction: "examples/lean_mcp_smoke_test/valid.leanを検証する",
                arguments: json!({}),
            },
        )
        .unwrap();
        assert_eq!(call.tool, "check_lean_code");
        assert!(call.arguments.get("path").is_none());
        let code =
            std::fs::read_to_string(config.resolve_path("examples/lean_mcp_smoke_test/valid.lean"))
                .unwrap();
        assert_eq!(call.arguments.get("code"), Some(&Value::String(code)));
    }

    #[test]
    fn prolog_adapter_selects_file_tool_from_typed_source() {
        let root = crate::test_support::project_root();
        let config = ProjectConfig::load(root).unwrap();
        let route = config.action("logic_rules").unwrap();
        let call = prepare_call(
            &config,
            AdapterInput {
                action: "logic_rules",
                route,
                requested_tool: None,
                input: "Prologルールを検査する",
                source_instruction: "file examples/open_data_population_pipeline/population_rules.pl, query `population_assessment_from_data(Trend, Attention).`",
                arguments: json!({}),
            },
        )
        .unwrap();
        assert_eq!(call.tool, "run_prolog");
        assert!(call.arguments.get("program_text").is_some());
    }

    #[test]
    fn normalizes_and_limits_mcp_output() {
        let result = json!({"content": [{"type": "text", "text": "123456789"}]});
        let normalized = normalize_result(AdapterKind::Prolog, "run_prolog", &result, 5).unwrap();
        assert!(normalized.truncated);
        assert_eq!(normalized.raw_bytes, 9);
        assert!(normalized.text.starts_with("12345"));
    }

    #[test]
    fn rejects_semantic_failure_inside_successful_mcp_transport() {
        let result = json!({
            "content": [{"type": "text", "text": "{\"ok\":false,\"error\":\"bad query\"}"}]
        });
        let error = normalize_result(AdapterKind::Prolog, "run_prolog", &result, 1024).unwrap_err();
        assert!(error.to_string().contains("bad query"));
    }

    #[test]
    fn codex_adapter_enforces_route_security_settings() {
        let root = crate::test_support::project_root();
        let config = ProjectConfig::load(root).unwrap();
        let route = config.action("open_data_acquisition").unwrap();
        let call = prepare_call(
            &config,
            AdapterInput {
                action: "open_data_acquisition",
                route,
                requested_tool: None,
                input: "fetch data",
                source_instruction: "fetch data",
                arguments: json!({"sandbox": "read-only"}),
            },
        )
        .unwrap();

        assert_eq!(
            call.arguments.get("sandbox"),
            Some(&Value::String("danger-full-access".to_owned()))
        );
        assert_eq!(
            call.arguments.get("approval-policy"),
            Some(&Value::String("never".to_owned()))
        );
        assert!(call.arguments.get("cwd").is_some());
    }

    #[test]
    fn matlab_adapter_recovers_code_from_original_instruction() {
        let root = crate::test_support::project_root();
        let config = ProjectConfig::load(root).unwrap();
        let route = config.action("calculation_graphing").unwrap();
        let call = prepare_call(
            &config,
            AdapterInput {
                action: "calculation_graphing",
                route,
                requested_tool: Some("evaluate_matlab_code"),
                input: "MATLABで解析する",
                source_instruction: "run(fullfile(pwd,'examples','open_data_population_pipeline','analyze_population.m'));",
                arguments: json!({}),
            },
        )
        .unwrap();
        assert_eq!(
            call.arguments.get("code"),
            Some(&Value::String(
                "run(fullfile(pwd,'examples','open_data_population_pipeline','analyze_population.m'));"
                    .to_owned()
            ))
        );
    }

    #[test]
    fn matlab_file_is_sent_as_code_after_the_capability_check() {
        let root = crate::test_support::project_root();
        let config = ProjectConfig::load(root).unwrap();
        let route = config.action("calculation_graphing").unwrap();
        let prepare = |tool: &str, path: &str| {
            prepare_call(
                &config,
                AdapterInput {
                    action: "calculation_graphing",
                    route,
                    requested_tool: Some(tool),
                    input: "",
                    source_instruction: "",
                    arguments: json!({"script_path": path}),
                },
            )
        };
        let call = prepare(
            "run_matlab_file",
            "examples/cvim_full_loop_test/full_loop_probe.m",
        )
        .unwrap();
        assert_eq!(call.tool, "evaluate_matlab_code");
        let code = call.arguments["code"].as_str().unwrap();
        assert!(code.contains("FULL_LOOP_MATLAB_OK"));
        assert!(call.arguments.get("script_path").is_none());
        // The file read still requires a file.read grant.
        assert!(prepare("run_matlab_file", "config/runtime.toml").is_err());
        assert!(
            prepare(
                "run_matlab_test_file",
                "examples/cvim_full_loop_test/full_loop_probe.m"
            )
            .is_err()
        );
    }

    #[test]
    fn explicit_matlab_tool_wins_over_file_suffix_in_source() {
        let root = crate::test_support::project_root();
        let config = ProjectConfig::load(root).unwrap();
        let route = config.action("calculation_graphing").unwrap();
        let call = prepare_call(
            &config,
            AdapterInput {
                action: "calculation_graphing",
                route,
                requested_tool: None,
                input: "",
                source_instruction: "Call evaluate_matlab_code with run(fullfile(pwd,'examples','analyze_population.m'));",
                arguments: json!({}),
            },
        )
        .unwrap();
        assert_eq!(call.tool, "evaluate_matlab_code");
    }
}
