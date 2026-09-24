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
            "kdb" | "run_q" => Ok(Self::Kdb),
            "filter" | "list_files" | "preview_file" => Ok(Self::Filter),
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
        AdapterKind::Kdb => prepare_kdb(&tool, &source, &mut arguments)?,
        AdapterKind::Filter => prepare_filter(config, &tool, &source, &mut arguments)?,
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
    if let Some(tool) = infer_tool_from_keywords(route, &source_text) {
        return Ok(tool);
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

/// Tools named by what the operator asks for rather than by a file type.
fn infer_tool_from_keywords(route: &ActionConfig, source_text: &str) -> Option<String> {
    const HINTS: [(&str, &[&str]); 2] = [
        (
            "list_files",
            &["一覧", "ファイル名", "list files", "listing"],
        ),
        ("preview_file", &["プレビュー", "中身", "先頭", "preview"]),
    ];
    HINTS
        .iter()
        .find(|(tool, words)| {
            route.preferred_tools.iter().any(|item| item == tool)
                && words.iter().any(|word| source_text.contains(word))
        })
        .map(|(tool, _)| (*tool).to_owned())
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
            let file = match required_path(object, "path", source, &[".lean"]) {
                Ok(file) => file,
                // A planner may pick the file tool for Lean written in the instruction.
                Err(error) => {
                    let code =
                        find_lean_code([source.source_instruction, source.input]).ok_or(error)?;
                    object.clear();
                    object.insert("code".to_owned(), Value::String(code));
                    return Ok(Some("check_lean_code".to_owned()));
                }
            };
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

        let is_declaration =
            |text: &str| DECLARATIONS.iter().any(|marker| text.starts_with(marker));
        if let Some(span) = code_spans(candidate)
            .into_iter()
            .find(|span| is_declaration(span))
        {
            return Some(span);
        }

        if let Some(start) = DECLARATIONS
            .iter()
            .filter_map(|marker| candidate.find(marker))
            .min()
        {
            // Source quoted inline ends at the closing backtick.
            let tail = &candidate[start..];
            let end = tail.find('`').unwrap_or(tail.len());
            return Some(tail[..end].trim().to_owned());
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
            if !object.contains_key("query")
                && let Some(inline) = find_inline_prolog([source.input, source.source_instruction])
            {
                object.insert("query".to_owned(), Value::String(inline.query));
                if !object.contains_key("program_text") {
                    object.insert("program_text".to_owned(), Value::String(inline.program));
                }
            }
            required_text(object, "query", source.input)?;
        }
        "run_prolog_file" => {
            let file = match required_path(object, "file", source, &[".pl", ".pro", ".prolog"]) {
                Ok(file) => file,
                // A planner may pick the file tool for a program written in the instruction.
                Err(error) => {
                    let inline = find_inline_prolog([source.input, source.source_instruction])
                        .ok_or(error)?;
                    object.clear();
                    object.insert("query".to_owned(), Value::String(inline.query));
                    object.insert("program_text".to_owned(), Value::String(inline.program));
                    return Ok(Some("run_prolog".to_owned()));
                }
            };
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
                    .or_else(|| {
                        find_code_span([source.input, source.source_instruction], |span| {
                            span.contains('(') || span.contains('=')
                        })
                    })
                    .or_else(|| (!has_cjk(source.input)).then(|| source.input.to_owned()))
                    .context("evaluate_matlab_code needs MATLAB code in arguments.code")?;
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

/// q code for `run_q`: the planner's, a backtick span, or plain-ASCII input.
/// Operator prose is never sent as code.
fn prepare_kdb(
    tool: &str,
    source: &AdapterInput<'_>,
    object: &mut Map<String, Value>,
) -> Result<Option<String>> {
    if tool == "run_q" && !object.contains_key("code") {
        let code = find_code_span([source.input, source.source_instruction], |span| {
            !span.contains('/') || span.contains(' ')
        })
        .or_else(|| (!has_cjk(source.input)).then(|| source.input.to_owned()))
        .context("run_q needs q code in arguments.code")?;
        required_text(object, "code", &code)?;
    }
    Ok(None)
}

/// Directory and file arguments for the Filter tools, from project paths
/// named in the instruction. A pipeline without stages is re-targeted to
/// listing or preview when the instruction asks for that.
fn prepare_filter(
    config: &ProjectConfig,
    tool: &str,
    source: &AdapterInput<'_>,
    object: &mut Map<String, Value>,
) -> Result<Option<String>> {
    let inputs = [source.input, source.source_instruction];
    let mut tool = tool.to_owned();
    let mut rewritten = None;
    if tool == "run_filter_pipeline" && !object.contains_key("stages") {
        let text = format!("{}\n{}", source.input, source.source_instruction).to_lowercase();
        // The planner's own arguments name the tool it meant.
        let from_arguments = if object.contains_key("path") {
            Some("preview_file".to_owned())
        } else if object.contains_key("subdir") {
            Some("list_files".to_owned())
        } else {
            None
        };
        if let Some(inferred) =
            from_arguments.or_else(|| infer_tool_from_keywords(source.route, &text))
        {
            tool = inferred.clone();
            rewritten = Some(inferred);
        }
    }
    match tool.as_str() {
        "list_files" if !object.contains_key("subdir") => {
            if let Some(dir) = find_project_entry(config, inputs, true) {
                object.insert("subdir".to_owned(), Value::String(dir));
            }
        }
        "preview_file" if !object.contains_key("path") => {
            let file = find_project_entry(config, inputs, false)
                .context("preview_file needs a project file path")?;
            object.insert("path".to_owned(), Value::String(file));
        }
        "csv_summary" | "group_by_count" if !object.contains_key("input_file") => {
            if let Some(file) = find_project_path(inputs, &[".csv"]) {
                object.insert("input_file".to_owned(), Value::String(file));
            }
        }
        _ => {}
    }
    Ok(rewritten)
}

/// The first project-relative path in the inputs that names an existing
/// directory (`want_dir`) or file.
fn find_project_entry<const N: usize>(
    config: &ProjectConfig,
    inputs: [&str; N],
    want_dir: bool,
) -> Option<String> {
    inputs.into_iter().find_map(|input| {
        let normalized = input.replace('\\', "/");
        let mut rest = normalized.as_str();
        while let Some(start) = ["examples/", ".runtime/"]
            .iter()
            .filter_map(|prefix| rest.find(prefix))
            .min()
        {
            let tail = &rest[start..];
            let end = tail
                .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-')))
                .unwrap_or(tail.len());
            let candidate = tail[..end].trim_end_matches(['.', '/']);
            let path = config.resolve_path(candidate);
            if !candidate.contains("..")
                && (if want_dir {
                    path.is_dir()
                } else {
                    path.is_file()
                })
            {
                return Some(candidate.to_owned());
            }
            rest = &tail[end.max(1)..];
        }
        None
    })
}

fn has_cjk(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xFF00..=0xFFEF)
    })
}
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

struct InlineProlog {
    program: String,
    query: String,
}

/// A Prolog program and query written as backtick spans in an instruction.
/// The query is the last span that is a single goal; the program is every
/// span holding clauses (rules, or several facts). A lone goal with no
/// program is still a valid query against built-ins.
fn find_inline_prolog<const N: usize>(inputs: [&str; N]) -> Option<InlineProlog> {
    let is_clauses =
        |span: &str| span.ends_with('.') && (span.contains(":-") || span.contains(". "));
    let is_goal = |span: &str| span.ends_with('.') && span.contains('(') && !is_clauses(span);
    let spans = inputs.map(code_spans);
    // The planner's input may carry only the query; the program then comes
    // from the operator instruction.
    let query = spans
        .iter()
        .find_map(|list| list.iter().rev().find(|span| is_goal(span)))?
        .clone();
    let program = spans
        .iter()
        .map(|list| {
            list.iter()
                .filter(|span| is_clauses(span))
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        })
        .find(|program| !program.is_empty())
        .unwrap_or_default();
    Some(InlineProlog { program, query })
}

/// Contents of the single-backtick spans in `input`, trimmed and non-empty.
fn code_spans(input: &str) -> Vec<String> {
    let mut spans = Vec::new();
    let mut remainder = input;
    while let Some(start) = remainder.find('`') {
        remainder = &remainder[start + 1..];
        let Some(end) = remainder.find('`') else {
            break;
        };
        let span = remainder[..end].trim();
        if !span.is_empty() {
            spans.push(span.to_owned());
        }
        remainder = &remainder[end + 1..];
    }
    spans
}

fn find_code_span<const N: usize>(
    inputs: [&str; N],
    predicate: impl Fn(&str) -> bool,
) -> Option<String> {
    inputs
        .into_iter()
        .find_map(|input| code_spans(input).into_iter().find(|span| predicate(span)))
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

    fn prepare(
        action: &str,
        tool: Option<&str>,
        input: &str,
        instruction: &str,
    ) -> Result<PreparedMcpCall> {
        let root = crate::test_support::project_root();
        let config = ProjectConfig::load(root).unwrap();
        let route = config.action(action).unwrap().clone();
        prepare_call(
            &config,
            AdapterInput {
                action,
                route: &route,
                requested_tool: tool,
                input,
                source_instruction: instruction,
                arguments: json!({}),
            },
        )
    }

    #[test]
    fn kdb_code_comes_from_a_backtick_span_never_from_prose() {
        let call = prepare(
            "table_analysis",
            Some("run_q"),
            "KDBで計算",
            "KDB/qで `sum til 101` を実行し、結果を答えてください。",
        )
        .unwrap();
        assert_eq!(call.adapter, AdapterKind::Kdb);
        assert_eq!(call.arguments["code"], "sum til 101");
        let error = prepare(
            "table_analysis",
            Some("run_q"),
            "KDBで平均を計算して",
            "KDBで平均を計算して",
        )
        .unwrap_err();
        assert!(error.to_string().contains("run_q needs q code"));
        let limits = prepare(
            "table_analysis",
            Some("list_server_limits"),
            "制限",
            "制限値を取得",
        )
        .unwrap();
        assert_eq!(limits.arguments, json!({}));
    }

    #[test]
    fn filter_tools_get_project_paths_and_keyword_tools() {
        let listing = prepare(
            "text_filtering",
            None,
            "一覧",
            "Filterで examples/lean_mcp_smoke_test ディレクトリのファイル一覧を取得してください。",
        )
        .unwrap();
        assert_eq!(listing.tool, "list_files");
        assert_eq!(listing.arguments["subdir"], "examples/lean_mcp_smoke_test");
        // A stage-less pipeline chosen by the planner is re-targeted.
        let preview = prepare(
            "text_filtering",
            Some("run_filter_pipeline"),
            "中身を見る",
            "Filterで examples/open_data_population_pipeline/population_rules.pl の中身をプレビューしてください。",
        )
        .unwrap();
        assert_eq!(preview.tool, "preview_file");
        let root = crate::test_support::project_root();
        let config = ProjectConfig::load(root).unwrap();
        let route = config.action("text_filtering").unwrap().clone();
        let with_subdir = prepare_call(
            &config,
            AdapterInput {
                action: "text_filtering",
                route: &route,
                requested_tool: Some("run_filter_pipeline"),
                input: "x",
                source_instruction: "x",
                arguments: json!({"subdir": "examples"}),
            },
        )
        .unwrap();
        assert_eq!(with_subdir.tool, "list_files");
        assert_eq!(
            preview.arguments["path"],
            "examples/open_data_population_pipeline/population_rules.pl"
        );
    }

    #[test]
    fn lean_inline_span_excludes_the_closing_backtick_and_prose() {
        let call = prepare(
            "logic_proof",
            Some("check_lean_code"),
            "Check the theorem",
            "Leanで次の定理が証明できるか検査してください: `theorem t (n : Nat) : n + 0 = n := by simp` 結果を答えて。",
        )
        .unwrap();
        assert_eq!(
            call.arguments["code"],
            "theorem t (n : Nat) : n + 0 = n := by simp"
        );
    }

    #[test]
    fn matlab_code_comes_from_a_span_and_prose_is_refused() {
        let call = prepare(
            "calculation_graphing",
            Some("evaluate_matlab_code"),
            "平均を計算",
            "MATLABで `mean([2 4 6 8])` を計算してください。",
        )
        .unwrap();
        assert_eq!(call.arguments["code"], "mean([2 4 6 8])");
        let error = prepare(
            "calculation_graphing",
            Some("evaluate_matlab_code"),
            "MATLABで1から100までの2乗和を計算",
            "MATLABで1から100までの2乗和を計算",
        )
        .unwrap_err();
        assert!(error.to_string().contains("needs MATLAB code"));
    }

    #[test]
    fn prolog_program_is_found_when_the_planner_input_has_only_the_query() {
        let call = prepare(
            "logic_rules",
            Some("run_prolog"),
            "`path(a,c).` を確認",
            "(1) Prologでプログラム `edge(a,b). edge(b,c). path(X,Y) :- edge(X,Y). path(X,Y) :- edge(X,Z), path(Z,Y).` に対し問い合わせ `path(a,c).` (2) Leanで `theorem two : 1 + 1 = 2 := by rfl`",
        )
        .unwrap();
        assert_eq!(call.arguments["query"], "path(a,c).");
        assert!(
            call.arguments["program_text"]
                .as_str()
                .unwrap()
                .contains("path(X,Y) :- edge(X,Y).")
        );
    }

    #[test]
    fn prolog_file_tool_without_a_file_uses_the_inline_program() {
        let root = crate::test_support::project_root();
        let config = ProjectConfig::load(root).unwrap();
        let route = config.action("logic_rules").unwrap();
        let instruction = "Prologで確認: プログラム: `edge(a,b). edge(b,c). path(X,Y) :- edge(X,Y). path(X,Y) :- edge(X,Z), path(Z,Y).` 問い合わせ: `path(a,c).`";
        for requested in ["run_prolog_file", "run_prolog"] {
            let call = prepare_call(
                &config,
                AdapterInput {
                    action: "logic_rules",
                    route,
                    requested_tool: Some(requested),
                    input: instruction,
                    source_instruction: instruction,
                    arguments: json!({}),
                },
            )
            .unwrap();
            assert_eq!(call.tool, "run_prolog", "requested {requested}");
            assert_eq!(call.arguments["query"], "path(a,c).");
            assert_eq!(
                call.arguments["program_text"],
                "edge(a,b). edge(b,c). path(X,Y) :- edge(X,Y). path(X,Y) :- edge(X,Z), path(Z,Y)."
            );
            assert!(call.arguments.get("file").is_none());
        }
    }

    #[test]
    fn prolog_file_tool_without_file_or_program_still_fails() {
        let root = crate::test_support::project_root();
        let config = ProjectConfig::load(root).unwrap();
        let route = config.action("logic_rules").unwrap();
        let error = prepare_call(
            &config,
            AdapterInput {
                action: "logic_rules",
                route,
                requested_tool: Some("run_prolog_file"),
                input: "Prologで何か確認する",
                source_instruction: "Prologで何か確認する",
                arguments: json!({}),
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("required path argument: file"));
    }

    #[test]
    fn lean_file_tool_without_a_file_uses_the_inline_source() {
        let root = crate::test_support::project_root();
        let config = ProjectConfig::load(root).unwrap();
        let route = config.action("logic_proof").unwrap();
        let call = prepare_call(
            &config,
            AdapterInput {
                action: "logic_proof",
                route,
                requested_tool: Some("check_lean_file"),
                input: "証明を確認",
                source_instruction: "次を検証: theorem t (n : Nat) : n = n := by rfl",
                arguments: json!({}),
            },
        )
        .unwrap();
        assert_eq!(call.tool, "check_lean_code");
        assert_eq!(
            call.arguments["code"],
            "theorem t (n : Nat) : n = n := by rfl"
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
