use crate::config::LaunchProfile;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::sleep;

#[derive(Debug, Clone, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    temperature: f32,
    stream: bool,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'a str>,
}

#[derive(Debug, Clone, Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Debug, Deserialize)]
struct ResponseMessage {
    content: serde_json::Value,
}

pub struct LlmClient {
    client: reqwest::Client,
}

impl LlmClient {
    pub fn new(timeout_sec: u64) -> Result<Self> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(timeout_sec))
            .build()?;
        Ok(Self { client })
    }

    pub async fn chat(
        &self,
        profile: &LaunchProfile,
        system: &str,
        user: &str,
        temperature: f32,
    ) -> Result<String> {
        self.chat_with_format(profile, system, user, temperature, 4096, None)
            .await
    }

    pub async fn chat_json(
        &self,
        profile: &LaunchProfile,
        system: &str,
        user: &str,
        temperature: f32,
        allowed_types: &[&str],
        allowed_actions: &[String],
    ) -> Result<String> {
        let action_values = if allowed_actions.is_empty() {
            vec!["__no_eligible_action__".to_owned()]
        } else {
            allowed_actions.to_vec()
        };
        self.chat_with_format(
            profile,
            system,
            user,
            temperature,
            1024,
            Some(serde_json::json!({
                "type": "json_schema",
                "json_schema": {
                    "name": "aiconductor_decision",
                    "strict": true,
                    "schema": decision_schema(allowed_types, &action_values),
                }
            })),
        )
        .await
    }

    async fn chat_with_format(
        &self,
        profile: &LaunchProfile,
        system: &str,
        user: &str,
        temperature: f32,
        max_tokens: u32,
        response_format: Option<serde_json::Value>,
    ) -> Result<String> {
        let model = profile.model_name()?;
        let reasoning_effort = if response_format.is_some() {
            profile
                .planner_reasoning_effort
                .as_deref()
                .or(profile.reasoning_effort.as_deref())
        } else {
            profile.reasoning_effort.as_deref()
        };
        let request = ChatRequest {
            model,
            messages: vec![
                ChatMessage {
                    role: "system",
                    content: system,
                },
                ChatMessage {
                    role: "user",
                    content: user,
                },
            ],
            temperature,
            stream: false,
            max_tokens,
            response_format,
            reasoning_effort,
        };
        let url = format!(
            "{}/v1/chat/completions",
            profile.api_base.trim_end_matches('/')
        );
        let request_bytes = serde_json::to_vec(&request)?.len();
        let mut completed_response = None;
        for attempt in 1..=3 {
            match self.client.post(&url).json(&request).send().await {
                Ok(response) => {
                    let status = response.status();
                    if status.is_success() {
                        completed_response = Some(response);
                        break;
                    } else {
                        let body = response
                            .text()
                            .await
                            .unwrap_or_else(|error| format!("failed to read error body: {error}"));
                        bail!(
                            "LLM request failed: HTTP {status} from {url} \
                             (request_bytes={request_bytes}): {}",
                            truncate(&body, 1000)
                        );
                    }
                }
                Err(error) => {
                    // The caller has no side effects until a complete response is received.
                    // Retry transient tunnel/server disconnects a small fixed number of times.
                    if attempt < 3
                        && (error.is_timeout() || error.is_connect() || error.is_request())
                    {
                        sleep(Duration::from_millis(250 * attempt)).await;
                        continue;
                    }
                    return Err(error.into());
                }
            }
        }
        let response = completed_response.context("LLM request exhausted retries")?;
        let payload: ChatResponse = response.json().await?;
        let content = &payload
            .choices
            .first()
            .context("LLM response has no choices")?
            .message
            .content;
        match content {
            serde_json::Value::String(text) => Ok(text.trim().to_owned()),
            serde_json::Value::Array(parts) => Ok(parts
                .iter()
                .filter_map(|part| part.get("text").and_then(|value| value.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
                .trim()
                .to_owned()),
            _ => anyhow::bail!("LLM response content has an unsupported shape"),
        }
    }
}

fn truncate(value: &str, limit: usize) -> String {
    let mut chars = value.chars();
    let shortened = chars.by_ref().take(limit).collect::<String>();
    if chars.next().is_some() {
        format!("{shortened}…")
    } else {
        shortened
    }
}

const DECISION_TOOLS: [&str; 24] = [
    "check_lean_code",
    "check_lean_file",
    "get_lean_environment",
    "run_prolog",
    "run_prolog_file",
    "evaluate_matlab_code",
    "run_matlab_file",
    "run_matlab_test_file",
    "run_q",
    "load_csv",
    "save_table",
    "save_csv",
    "load_table",
    "get_interpreter_state",
    "get_state_source",
    "prune_state",
    "list_server_limits",
    "run_filter_pipeline",
    "group_by_count",
    "csv_summary",
    "preview_file",
    "list_files",
    "list_allowed_commands",
    "codex",
];

/// Decision schema. When only one decision type is allowed (a decider chose
/// it), that type's keys are all required, so a dispatch carries arguments.
fn decision_schema(allowed_types: &[&str], actions: &[String]) -> serde_json::Value {
    let mut tools = vec![serde_json::Value::Null];
    tools.extend(DECISION_TOOLS.iter().map(|tool| serde_json::json!(tool)));
    let dispatch = serde_json::json!({
        "type": "object",
        "properties": {
            "type": {"const": "dispatch"},
            "action": {"type": "string", "enum": actions},
            "input": {"type": "string"},
            "tool": {"enum": tools},
            "arguments": {
                "type": "object",
                "properties": {
                    "code": {"type": "string"},
                    "query": {"type": "string"},
                    "program_text": {"type": "string"},
                    "file": {"type": "string"},
                    "path": {"type": "string"},
                    "script_path": {"type": "string"},
                    "subdir": {"type": "string"}
                },
                "additionalProperties": false
            }
        },
        "required": ["type", "action", "input", "tool", "arguments"],
        "additionalProperties": false
    });
    let complete = serde_json::json!({
        "type": "object",
        "properties": {
            "type": {"const": "complete"},
            "answer": {"type": "string"},
            "polish_japanese": {"type": "boolean"}
        },
        "required": ["type", "answer", "polish_japanese"],
        "additionalProperties": false
    });
    match allowed_types {
        ["dispatch"] => dispatch,
        ["complete"] => complete,
        // Both allowed: one object whose only required key is `type`. Keys are
        // emitted required-first, so the model states dispatch or complete
        // before anything else. With every key required (or one branch per
        // type), keys come out alphabetically and "action" precedes "type";
        // models then committed to dispatching and never completed.
        _ => serde_json::json!({
            "type": "object",
            "properties": {
                "type": {"type": "string", "enum": allowed_types},
                "action": dispatch["properties"]["action"],
                "input": {"type": "string"},
                "tool": dispatch["properties"]["tool"],
                "arguments": dispatch["properties"]["arguments"],
                "answer": {"type": "string"},
                "polish_japanese": {"type": "boolean"}
            },
            "required": ["type"],
            "additionalProperties": false
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_decisions_ask_for_the_type_first() {
        let actions = vec!["logic_rules".to_owned()];
        let open = decision_schema(&["dispatch", "complete"], &actions);
        assert_eq!(open["required"], serde_json::json!(["type"]));
        assert!(open.get("anyOf").is_none());
        assert_eq!(
            open["properties"]["action"]["enum"],
            serde_json::json!(["logic_rules"])
        );
        assert!(open["properties"]["arguments"]["properties"]["code"].is_object());
        // A decider-scoped dispatch requires its arguments.
        let dispatch = decision_schema(&["dispatch"], &actions);
        let required = dispatch["required"].as_array().unwrap();
        assert!(required.contains(&serde_json::json!("arguments")));
        let complete = decision_schema(&["complete"], &actions);
        assert_eq!(complete["properties"]["type"]["const"], "complete");
    }
}
