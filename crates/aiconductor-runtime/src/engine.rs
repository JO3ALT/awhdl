use crate::adapters::{AdapterInput, normalize_result, prepare_call};
use crate::audit::{RunState, RunStore};
use crate::budget::Budget;
use crate::config::{ActionConfig, ProjectConfig};
use crate::llm::LlmClient;
use crate::mcp::McpClient;
use crate::model::ModelManager;
use crate::state_machine::{ActionStateMachine, TransitionError};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
struct Observation {
    action: String,
    ok: bool,
    result: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Decision {
    Dispatch {
        action: String,
        #[serde(default)]
        input: String,
        #[serde(default)]
        tool: Option<String>,
        #[serde(default)]
        arguments: Map<String, Value>,
    },
    Complete {
        answer: String,
        #[serde(default)]
        polish_japanese: bool,
    },
}

struct ActionInvocation<'a> {
    name: &'a str,
    route: &'a ActionConfig,
    input: &'a str,
    source_instruction: &'a str,
    requested_tool: Option<&'a str>,
    arguments: Value,
}

pub struct Engine {
    config: ProjectConfig,
    models: ModelManager,
    llm: LlmClient,
}

impl Engine {
    pub fn new(config: ProjectConfig) -> Result<Self> {
        let timeout = config.runtime.loop_limits.device_timeout_sec;
        Ok(Self {
            config,
            models: ModelManager::new()?,
            llm: LlmClient::new(timeout)?,
        })
    }

    pub async fn run(&self, prompt: &str) -> Result<String> {
        if prompt.trim().is_empty() {
            bail!("instruction is empty");
        }
        let mut audit = RunStore::create(&self.config.root, &self.config.runtime.run_root, prompt)?;
        let mut budget = Budget::new(self.config.runtime.loop_limits.clone());
        audit.event(
            "run_started",
            json!({"prompt_bytes": prompt.len()}),
            &budget.usage,
        )?;

        let outcome = async {
            let mut observations = Vec::<Observation>::new();
            let workflow = self
                .config
                .routes
                .select_workflow(prompt)
                .map(|(name, workflow)| (name.to_owned(), workflow.steps.clone()));
            if let Some((name, steps)) = &workflow {
                audit.event(
                    "workflow_selected",
                    json!({"workflow": name, "steps": steps}),
                    &budget.usage,
                )?;
            }
            let mut state_machine = ActionStateMachine::new(
                &self.config.routes.actions,
                &self.config.routes.default_action,
                workflow.as_ref().map(|(_, steps)| steps.as_slice()),
            );
            let mut successful_mcp_calls = BTreeMap::<String, String>::new();
            loop {
                budget.iteration()?;
                checkpoint(&audit, "planning", None, &budget)?;
                let decision = if workflow.is_some()
                    && let Some(action) = state_machine.next_workflow_action()?
                {
                    let input = scope_action_instruction(prompt, &action);
                    audit.event(
                        "state_machine_decision",
                        json!({"action": action}),
                        &budget.usage,
                    )?;
                    Decision::Dispatch {
                        action,
                        input,
                        tool: None,
                        arguments: Map::new(),
                    }
                } else {
                    self.plan(
                        prompt,
                        &observations,
                        &state_machine,
                        &mut budget,
                        &mut audit,
                    )
                    .await?
                };
                match decision {
                    Decision::Dispatch {
                        action,
                        input,
                        tool,
                        arguments,
                    } => {
                        if action == self.config.routes.default_action {
                            observations.push(Observation {
                                action,
                                ok: false,
                                result: "The controller action cannot dispatch itself.".to_owned(),
                            });
                            continue;
                        }
                        let route = self
                            .config
                            .action(&action)
                            .with_context(|| format!("planner selected unknown action: {action}"))?
                            .clone();
                        match state_machine.start(&action) {
                            Ok(attempt) => {
                                audit.event(
                                    "action_started",
                                    json!({"action": action, "attempt": attempt}),
                                    &budget.usage,
                                )?;
                            }
                            Err(error) => {
                                let already_succeeded =
                                    matches!(error, TransitionError::AlreadySucceeded(_));
                                audit.event(
                                    if already_succeeded {
                                        "duplicate_action_suppressed"
                                    } else {
                                        "action_transition_rejected"
                                    },
                                    json!({"action": action, "error": error.to_string()}),
                                    &budget.usage,
                                )?;
                                observations.push(Observation {
                                    action,
                                    ok: already_succeeded,
                                    result: format!(
                                        "STATE_MACHINE_REJECTED: {error}. Select a different eligible action or complete the run."
                                    ),
                                });
                                continue;
                            }
                        }
                        audit.event(
                            "route_resolved",
                            json!({"action": action, "kind": route.kind, "tool": tool}),
                            &budget.usage,
                        )?;
                        checkpoint(&audit, "executing", Some(&action), &budget)?;
                        let effective_input = if input.trim().is_empty() {
                            prompt
                        } else {
                            &input
                        };
                        let result = self
                            .execute_action(
                                ActionInvocation {
                                    name: &action,
                                    route: &route,
                                    input: effective_input,
                                    source_instruction: prompt,
                                    requested_tool: tool.as_deref(),
                                    arguments: Value::Object(arguments),
                                },
                                &mut budget,
                                &mut audit,
                                &mut successful_mcp_calls,
                            )
                            .await;
                        match result {
                            Ok(result) => {
                                state_machine.succeed(&action)?;
                                audit.event(
                                    "action_completed",
                                    json!({"action": action, "result_bytes": result.len()}),
                                    &budget.usage,
                                )?;
                                observations.push(Observation {
                                    action,
                                    ok: true,
                                    result: truncate_owned(
                                        result,
                                        self.config.runtime.planner.max_observation_bytes,
                                    ),
                                });
                            }
                            Err(error) => {
                                let message = format!("{error:#}");
                                state_machine.fail(&action, message.clone())?;
                                audit.event(
                                    "action_failed",
                                    json!({"action": action, "error": message}),
                                    &budget.usage,
                                )?;
                                observations.push(Observation {
                                    action,
                                    ok: false,
                                    result: message,
                                });
                            }
                        }
                    }
                    Decision::Complete {
                        mut answer,
                        polish_japanese,
                    } => {
                        if answer.trim().is_empty() {
                            observations.push(Observation {
                                action: "complete".to_owned(),
                                ok: false,
                                result: "Completion answer was empty.".to_owned(),
                            });
                            continue;
                        }
                        if polish_japanese
                            && let Some(action_name) = self
                                .config
                                .routes
                                .presentation_pipeline
                                .get("japanese_polishing")
                        {
                            let route = self
                                .config
                                .action(action_name)
                                .context("Japanese polishing route is missing")?
                                .clone();
                            state_machine.start(action_name)?;
                            let polished = self
                                .execute_action(
                                    ActionInvocation {
                                        name: action_name,
                                        route: &route,
                                        input: &answer,
                                        source_instruction: prompt,
                                        requested_tool: None,
                                        arguments: Value::Object(Map::new()),
                                    },
                                    &mut budget,
                                    &mut audit,
                                    &mut successful_mcp_calls,
                                )
                                .await;
                            match polished {
                                Ok(value) => {
                                    state_machine.succeed(action_name)?;
                                    answer = value;
                                }
                                Err(error) => {
                                    state_machine.fail(action_name, format!("{error:#}"))?;
                                    return Err(error);
                                }
                            }
                        }
                        audit.write_result(&answer)?;
                        audit.event(
                            "run_completed",
                            json!({
                                "result_bytes": answer.len(),
                                "executed_actions": state_machine.succeeded_actions(),
                                "polished": polish_japanese
                            }),
                            &budget.usage,
                        )?;
                        checkpoint(&audit, "completed", None, &budget)?;
                        return Ok(answer);
                    }
                }
            }
        }
        .await;
        if let Err(error) = &outcome {
            let message = format!("{error:#}");
            audit.event("run_failed", json!({"error": message}), &budget.usage)?;
            checkpoint(&audit, "failed", None, &budget)?;
        }
        outcome
    }

    async fn plan(
        &self,
        prompt: &str,
        observations: &[Observation],
        state_machine: &ActionStateMachine,
        budget: &mut Budget,
        audit: &mut RunStore,
    ) -> Result<Decision> {
        let controller_id = &self.config.models.default_orchestrator;
        let profile = self
            .models
            .ensure(controller_id, &self.config, budget, audit)
            .await?;
        let system = self.planner_system_prompt(state_machine);
        let eligible_actions = state_machine.eligible_actions();
        let context = json!({
            "instruction": prompt,
            "observations": observations,
            "action_states": state_machine.snapshot(),
            "remaining_iterations": self.config.runtime.loop_limits.max_iterations
                .saturating_sub(budget.usage.iterations),
        });
        let mut user = serde_json::to_string_pretty(&context)?;
        let attempts = self.config.runtime.loop_limits.planner_retries + 1;
        for attempt in 1..=attempts {
            budget.model_call()?;
            audit.event(
                "planner_request_started",
                json!({"attempt": attempt, "input_bytes": user.len()}),
                &budget.usage,
            )?;
            let raw = self
                .llm
                .chat_json(
                    &profile,
                    &system,
                    &user,
                    self.config.runtime.planner.temperature,
                    &eligible_actions,
                )
                .await?;
            match parse_decision(&raw) {
                Ok(decision) => {
                    audit.event(
                        "planner_decision",
                        json!({"attempt": attempt, "decision": raw}),
                        &budget.usage,
                    )?;
                    return Ok(decision);
                }
                Err(error) if attempt < attempts => {
                    audit.event(
                        "planner_invalid_json",
                        json!({"attempt": attempt, "error": error.to_string(), "output": raw}),
                        &budget.usage,
                    )?;
                    user = format!(
                        "The previous output was invalid: {error}. Return one valid JSON object only.\nPrevious output:\n{raw}"
                    );
                }
                Err(error) => return Err(error.context("planner did not return a valid decision")),
            }
        }
        unreachable!()
    }

    fn planner_system_prompt(&self, state_machine: &ActionStateMachine) -> String {
        let eligible = state_machine.eligible_actions();
        let actions = self
            .config
            .routes
            .actions
            .iter()
            .filter(|(name, _)| *name != &self.config.routes.default_action)
            .filter(|(name, _)| eligible.contains(name))
            .map(|(name, action)| {
                json!({
                    "name": name,
                    "description": action.description,
                    "kind": action.kind,
                    "tool": action.tool,
                    "preferred_tools": action.preferred_tools,
                })
            })
            .collect::<Vec<_>>();
        format!(
            r#"You are the planning device for AIConductor. You do not execute tools.
Choose exactly one next state and output one JSON object with no Markdown.

Dispatch schema:
{{"type":"dispatch","action":"ACTION_ID","input":"task for device","tool":null,"arguments":{{}}}}

Completion schema:
{{"type":"complete","answer":"final operator-facing answer","polish_japanese":false}}

Rules:
- Use only an ACTION_ID listed below. Never dispatch the loop/controller action.
- If Available actions is empty, return the completion schema.
- Base the next decision on the original instruction and all observations.
- For an MCP action with a fixed tool, tool may be null. For preferred tools, select one listed tool.
- The tool field is only the exact tool name. Put tool parameters in arguments, never inside tool.
- For check_lean_code put only compilable Lean source in arguments.code, with no prose or request wording.
- For evaluate_matlab_code put executable MATLAB text in arguments.code with no prose.
- For run_prolog_file put the source path in arguments.file and the query in arguments.query.
- arguments must be a JSON object containing typed MCP arguments when known.
- Treat structured adapter output as immutable facts. Copy numeric values exactly and never rescale them.
- Do not repeat a successful action unless its result shows another pass is needed.
- A failed observation may be retried with corrected input or routed to a fallback.
- Complete when the operator's request is answered. Set polish_japanese true only for Japanese prose that benefits from final editing.

Available actions:
{}"#,
            serde_json::to_string_pretty(&actions).unwrap_or_default()
        )
    }

    async fn execute_action(
        &self,
        invocation: ActionInvocation<'_>,
        budget: &mut Budget,
        audit: &mut RunStore,
        successful_mcp_calls: &mut BTreeMap<String, String>,
    ) -> Result<String> {
        let ActionInvocation {
            name: action_name,
            route,
            input,
            source_instruction,
            requested_tool,
            arguments,
        } = invocation;
        match route.kind.as_str() {
            "model" => {
                let primary = route.model.as_deref().context("model route has no model")?;
                let mut candidates = vec![primary];
                candidates.extend(route.fallback.iter().map(String::as_str));
                let mut last_error = None;
                for profile_id in candidates {
                    let profile = match self
                        .models
                        .ensure(profile_id, &self.config, budget, audit)
                        .await
                    {
                        Ok(profile) => profile,
                        Err(error) => {
                            last_error = Some(error);
                            continue;
                        }
                    };
                    budget.model_call()?;
                    let system = if action_name == "japanese_polishing" {
                        "与えられた文章の意味、事実、コード、数値を変えず、自然で明確な日本語に清書してください。清書後の本文だけを返してください。"
                    } else {
                        &route.description
                    };
                    match self.llm.chat(&profile, system, input, 0.2).await {
                        Ok(value) => return Ok(value),
                        Err(error) => last_error = Some(error),
                    }
                }
                Err(last_error.context("all model candidates failed")?)
            }
            "mcp" => {
                let server_name = route.server.as_deref().context("MCP route has no server")?;
                let server = self
                    .config
                    .mcp
                    .servers
                    .get(server_name)
                    .with_context(|| format!("unknown MCP server: {server_name}"))?;
                let prepared = prepare_call(
                    &self.config,
                    AdapterInput {
                        action: action_name,
                        route,
                        requested_tool,
                        input,
                        source_instruction,
                        arguments,
                    },
                )?;
                let fingerprint = serde_json::to_string(&json!({
                    "server": server_name,
                    "tool": prepared.tool,
                    "arguments": prepared.arguments,
                }))?;
                if let Some(previous) = successful_mcp_calls.get(&fingerprint) {
                    audit.event(
                        "duplicate_action_suppressed",
                        json!({"action": action_name, "tool": prepared.tool}),
                        &budget.usage,
                    )?;
                    return Ok(format!(
                        "DUPLICATE_ACTION_SUPPRESSED: This exact MCP call already succeeded. Do not dispatch it again; continue to the next step or complete.\nPrevious result:\n{previous}"
                    ));
                }
                audit.event(
                    "mcp_call_prepared",
                    json!({
                        "action": action_name,
                        "adapter": prepared.adapter,
                        "tool": prepared.tool,
                        "argument_keys": prepared.arguments
                            .as_object()
                            .map(|object| object.keys().collect::<Vec<_>>())
                            .unwrap_or_default(),
                    }),
                    &budget.usage,
                )?;
                budget.mcp_call()?;
                let result = McpClient::call_tool(
                    &self.config,
                    server,
                    &prepared.tool,
                    prepared.arguments.clone(),
                )
                .await?;
                let result = normalize_result(
                    prepared.adapter,
                    &prepared.tool,
                    &result,
                    self.config.runtime.planner.max_observation_bytes,
                )?
                .for_planner()?;
                successful_mcp_calls.insert(fingerprint, result.clone());
                Ok(result)
            }
            other => bail!("unsupported route kind: {other}"),
        }
    }
}

fn checkpoint(audit: &RunStore, phase: &str, action: Option<&str>, budget: &Budget) -> Result<()> {
    audit.checkpoint(&RunState {
        run_id: audit.id(),
        phase: phase.to_owned(),
        iteration: budget.usage.iterations,
        last_action: action.map(str::to_owned),
        budget: budget.usage.clone(),
    })
}

fn scope_action_instruction(prompt: &str, action: &str) -> String {
    let marker = format!("`{action}`");
    let Some(marker_position) = prompt.find(&marker) else {
        return prompt.to_owned();
    };
    let section_start = prompt[..marker_position]
        .rfind("\n## ")
        .map_or(0, |position| position + 1);
    let section_end = prompt[marker_position..]
        .find("\n## ")
        .map_or(prompt.len(), |position| marker_position + position);
    prompt[section_start..section_end].trim().to_owned()
}

fn parse_decision(raw: &str) -> Result<Decision> {
    let trimmed = raw.trim();
    let json_text = if trimmed.starts_with("```") {
        trimmed
            .strip_prefix("```json")
            .or_else(|| trimmed.strip_prefix("```"))
            .and_then(|value| value.strip_suffix("```"))
            .map(str::trim)
            .context("incomplete fenced JSON")?
    } else {
        trimmed
    };
    serde_json::from_str(json_text).context("invalid planner decision JSON")
}

fn truncate_owned(mut value: String, max: usize) -> String {
    if value.len() <= max {
        return value;
    }
    let mut end = max;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    value.push_str("\n[observation truncated]");
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_and_fenced_decisions() {
        let plain = r#"{"type":"complete","answer":"ok","polish_japanese":false}"#;
        assert!(matches!(
            parse_decision(plain).unwrap(),
            Decision::Complete { .. }
        ));
        let fenced = format!("```json\n{plain}\n```");
        assert!(matches!(
            parse_decision(&fenced).unwrap(),
            Decision::Complete { .. }
        ));
    }

    #[test]
    fn scopes_a_deterministic_action_to_its_markdown_section() {
        let prompt = "# test\n\n## One\nCall `first` now.\n\n## Two\nCall `second` later.";
        assert_eq!(
            scope_action_instruction(prompt, "first"),
            "## One\nCall `first` now."
        );
    }
}
