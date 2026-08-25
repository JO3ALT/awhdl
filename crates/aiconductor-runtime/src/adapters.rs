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
            "claude_review" | "review_code" => Ok(Self::ClaudeReview),
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
    let tool = resolve_tool(source.route, source.requested_tool, &source)?;
    let adapter = AdapterKind::resolve(source.route, &tool)?;
    let mut arguments = source.arguments.as_object().cloned().unwrap_or_default();

    match adapter {
        AdapterKind::Codex => prepare_codex(config, &source, &mut arguments)?,
        AdapterKind::ClaudeReview => prepare_claude_review(&source, &mut arguments),
        AdapterKind::Lean => prepare_lean(config, &tool, &source, &mut arguments)?,
        AdapterKind::Prolog => prepare_prolog(config, &tool, &source, &mut arguments)?,
        AdapterKind::Matlab => prepare_matlab(config, &tool, &source, &mut arguments)?,
        AdapterKind::Passthrough => {}
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

fn prepare_codex(
    config: &ProjectConfig,
    source: &AdapterInput<'_>,
    object: &mut Map<String, Value>,
) -> Result<()> {
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

fn prepare_claude_review(source: &AdapterInput<'_>, object: &mut Map<String, Value>) {
    object
        .entry("instructions".to_owned())
        .or_insert_with(|| Value::String(effective_input(source).to_owned()));
    object
        .entry("paths".to_owned())
        .or_insert_with(|| Value::Array(Vec::new()));
}

fn effective_input<'a>(source: &'a AdapterInput<'_>) -> &'a str {
    if source.input.trim().is_empty() {
        source.source_instruction
    } else {
        source.input
    }
}

fn prepare_lean(
    config: &ProjectConfig,
    tool: &str,
    source: &AdapterInput<'_>,
    object: &mut Map<String, Value>,
) -> Result<()> {
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
            ensure_readable_project_file(config, &file)?;
            object.insert(
                "path".to_owned(),
                Value::String(config.resolve_path(&file).to_string_lossy().into_owned()),
            );
        }
        "get_lean_environment" => {}
        _ => bail!("Lean adapter does not support tool {tool}"),
    }
    Ok(())
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
) -> Result<()> {
    match tool {
        "run_prolog" => {
            required_text(object, "query", source.input)?;
        }
        "run_prolog_file" => {
            let file = required_path(object, "file", source, &[".pl", ".pro", ".prolog"])?;
            ensure_readable_project_file(config, &file)?;
            if !object.contains_key("query")
                && let Some(query) =
                    find_code_span([source.input, source.source_instruction], |value| {
                        value.contains("(") && value.ends_with('.')
                    })
            {
                object.insert("query".to_owned(), Value::String(query));
            }
        }
        _ => bail!("Prolog adapter does not support tool {tool}"),
    }
    Ok(())
}

fn prepare_matlab(
    config: &ProjectConfig,
    tool: &str,
    source: &AdapterInput<'_>,
    object: &mut Map<String, Value>,
) -> Result<()> {
    match tool {
        "evaluate_matlab_code" => {
            if !object.contains_key("code") {
                let code = find_matlab_run([source.input, source.source_instruction])
                    .unwrap_or_else(|| source.input.to_owned());
                required_text(object, "code", &code)?;
            }
        }
        "run_matlab_file" | "run_matlab_test_file" => {
            let file = required_path(object, "script_path", source, &[".m"])?;
            ensure_readable_project_file(config, &file)?;
        }
        _ => bail!("MATLAB adapter does not support tool {tool}"),
    }
    Ok(())
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

fn ensure_readable_project_file(config: &ProjectConfig, value: &str) -> Result<()> {
    let path = config.resolve_path(value);
    if !path.is_file() {
        bail!("adapter input file does not exist: {}", path.display());
    }
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
    use std::path::Path;

    #[test]
    fn prolog_adapter_recovers_typed_file_and_query() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
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
        assert_eq!(
            call.arguments.get("file"),
            Some(&Value::String(
                "examples/open_data_population_pipeline/population_rules.pl".to_owned()
            ))
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
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
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
    fn lean_file_adapter_passes_an_absolute_verified_path() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
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
        let path = call.arguments.get("path").and_then(Value::as_str).unwrap();
        assert!(Path::new(path).is_absolute());
        assert!(Path::new(path).is_file());
    }

    #[test]
    fn prolog_adapter_selects_file_tool_from_typed_source() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
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
        assert_eq!(call.tool, "run_prolog_file");
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
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
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
    fn fixed_tool_rejects_override() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let config = ProjectConfig::load(root).unwrap();
        let route = config.action("code_review").unwrap();
        let error = prepare_call(
            &config,
            AdapterInput {
                action: "code_review",
                route,
                requested_tool: Some("codex"),
                input: "review",
                source_instruction: "review",
                arguments: json!({}),
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("not allowed"));
    }

    #[test]
    fn matlab_adapter_recovers_code_from_original_instruction() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
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
    fn explicit_matlab_tool_wins_over_file_suffix_in_source() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
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
