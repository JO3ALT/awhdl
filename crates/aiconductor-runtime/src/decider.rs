//! Decision-model controller: picks the next action as a calibrated choice.
//!
//! A System One style decision model (for example autotrust/JEV) returns a
//! probability per option instead of text. It only selects the next action;
//! the LLM planner then fills that action's input and arguments. Low
//! confidence hands the whole decision back to the LLM planner.
use crate::config::DeciderConfig;
use crate::state_machine::ActionState;
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use std::time::Duration;

/// Option label that ends the run instead of dispatching an action.
pub const COMPLETE_OPTION: &str = "complete";
/// Decision models of this kind accept 2 to 16 options per choice.
pub const MAX_OPTIONS: usize = 16;

const QUESTION: &str = "Which step should the orchestrator run next to fulfil the operator instruction? \
Choose complete only when every required step has succeeded.";

#[derive(Debug, Serialize)]
struct DecisionRequest<'a> {
    kind: &'static str,
    state: &'a str,
    question: &'a str,
    options: &'a [String],
    truncate: bool,
}

#[derive(Debug, Deserialize)]
struct DecisionResponse {
    distribution: Vec<f64>,
    #[serde(default)]
    confidence: Option<f64>,
}

/// One candidate for the next step, as offered to the decision model.
#[derive(Debug, Clone, PartialEq)]
pub struct DeciderOption {
    /// Action ID, or [`COMPLETE_OPTION`].
    pub action: String,
    /// Text shown to the model.
    pub label: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeciderOutcome {
    pub action: String,
    pub confidence: f64,
    pub distribution: Vec<f64>,
}

impl DeciderOutcome {
    pub fn is_complete(&self) -> bool {
        self.action == COMPLETE_OPTION
    }
}

/// A past step, reduced to what the decision model needs.
pub struct StepSummary<'a> {
    pub action: &'a str,
    pub ok: bool,
    pub result: &'a str,
}

pub struct DecisionClient {
    client: reqwest::Client,
    config: DeciderConfig,
}

impl DecisionClient {
    pub fn new(config: DeciderConfig) -> Result<Self> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(config.timeout_sec))
            .build()?;
        Ok(Self { client, config })
    }

    pub fn config(&self) -> &DeciderConfig {
        &self.config
    }

    pub async fn decide(&self, state: &str, options: &[DeciderOption]) -> Result<DeciderOutcome> {
        ensure!(
            (2..=MAX_OPTIONS).contains(&options.len()),
            "decision model needs 2..={MAX_OPTIONS} options, got {}",
            options.len()
        );
        let labels = options
            .iter()
            .map(|option| option.label.clone())
            .collect::<Vec<_>>();
        let response = self
            .client
            .post(&self.config.endpoint)
            .json(&DecisionRequest {
                kind: "choice",
                state,
                question: QUESTION,
                options: &labels,
                truncate: true,
            })
            .send()
            .await
            .context("decision model request failed")?
            .error_for_status()
            .context("decision model returned an error status")?
            .json::<DecisionResponse>()
            .await
            .context("decision model returned an invalid response")?;
        interpret(response, options)
    }
}

/// Map the returned distribution back to an action. The runtime takes the
/// argmax itself rather than trusting a label echoed by the service.
fn interpret(response: DecisionResponse, options: &[DeciderOption]) -> Result<DeciderOutcome> {
    let distribution = response.distribution;
    if distribution.len() != options.len() {
        bail!(
            "decision model returned {} probabilities for {} options",
            distribution.len(),
            options.len()
        );
    }
    if distribution
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0)
    {
        bail!("decision model returned an invalid probability");
    }
    let total = distribution.iter().sum::<f64>();
    if (total - 1.0).abs() > 0.01 {
        bail!("decision model probabilities sum to {total}, not 1");
    }
    let (index, top) = distribution
        .iter()
        .copied()
        .enumerate()
        .max_by(|left, right| left.1.total_cmp(&right.1))
        .context("decision model returned no probabilities")?;
    // A calibrated model's own confidence is preferred; the top probability is
    // the fallback. Either way it must be a probability.
    let confidence = response.confidence.unwrap_or(top);
    ensure!(
        (0.0..=1.0).contains(&confidence),
        "decision model confidence {confidence} is outside 0..=1"
    );
    Ok(DeciderOutcome {
        action: options[index].action.clone(),
        confidence,
        distribution,
    })
}

/// Eligible actions plus completion, as decision-model options. `None` when
/// there is nothing to decide or more options than the model accepts.
pub fn build_options(
    eligible: &[String],
    describe: impl Fn(&str) -> String,
) -> Option<Vec<DeciderOption>> {
    if eligible.is_empty() || eligible.len() + 1 > MAX_OPTIONS {
        return None;
    }
    let mut options = eligible
        .iter()
        .map(|action| DeciderOption {
            action: action.clone(),
            label: format!("{action}: {}", describe(action)),
        })
        .collect::<Vec<_>>();
    options.push(DeciderOption {
        action: COMPLETE_OPTION.to_owned(),
        label: format!("{COMPLETE_OPTION}: finish the run and report the results"),
    });
    Some(options)
}

/// A compact state for a decision model with a short context window. Action
/// states come first because they matter most; the instruction head and the
/// most recent step results fill the remaining budget.
pub fn build_state(
    instruction: &str,
    action_states: &[ActionState],
    steps: &[StepSummary<'_>],
    max_chars: usize,
) -> String {
    let mut state = String::from("Action states:\n");
    for item in action_states {
        let _ = writeln!(
            state,
            "- {}: {} (attempts {}/{})",
            item.action,
            serde_json::to_value(item.status)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_default(),
            item.attempts,
            item.max_attempts
        );
    }
    if !steps.is_empty() {
        state.push_str("Recent results:\n");
        for step in steps.iter().rev().take(4).rev() {
            let _ = writeln!(
                state,
                "- {} {}: {}",
                step.action,
                if step.ok { "ok" } else { "failed" },
                truncate_chars(&one_line(step.result), 160)
            );
        }
    }
    state.push_str("Instruction:\n");
    let remaining = max_chars.saturating_sub(state.chars().count());
    state.push_str(&truncate_chars(instruction.trim(), remaining));
    truncate_chars(&state, max_chars)
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((index, _)) => text[..index].to_owned(),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_machine::ActionStatus;

    fn options(actions: &[&str]) -> Vec<DeciderOption> {
        build_options(
            &actions.iter().map(|a| a.to_string()).collect::<Vec<_>>(),
            |a| format!("does {a}"),
        )
        .unwrap()
    }

    fn state(action: &str, status: ActionStatus) -> ActionState {
        ActionState {
            action: action.to_owned(),
            status,
            attempts: 0,
            max_attempts: 2,
            repeatable: false,
            depends_on: Vec::new(),
            last_error: None,
        }
    }

    #[test]
    fn options_append_completion_and_respect_the_model_limit() {
        let built = options(&["logic_rules", "logic_proof"]);
        assert_eq!(built.len(), 3);
        assert_eq!(built[2].action, COMPLETE_OPTION);
        assert_eq!(built[0].label, "logic_rules: does logic_rules");
        assert!(build_options(&[], |_| String::new()).is_none());
        let many = (0..MAX_OPTIONS)
            .map(|i| format!("a{i}"))
            .collect::<Vec<_>>();
        assert!(build_options(&many, |_| String::new()).is_none());
    }

    #[test]
    fn argmax_of_distribution_selects_the_action() {
        let outcome = interpret(
            DecisionResponse {
                distribution: vec![0.1, 0.7, 0.2],
                confidence: None,
            },
            &options(&["logic_rules", "logic_proof"]),
        )
        .unwrap();
        assert_eq!(outcome.action, "logic_proof");
        assert!((outcome.confidence - 0.7).abs() < 1e-9);
        assert!(!outcome.is_complete());
    }

    #[test]
    fn malformed_distributions_are_rejected() {
        let opts = options(&["logic_rules", "logic_proof"]);
        let bad = |distribution: Vec<f64>, confidence| {
            interpret(
                DecisionResponse {
                    distribution,
                    confidence,
                },
                &opts,
            )
            .is_err()
        };
        assert!(bad(vec![0.5, 0.5], None));
        assert!(bad(vec![0.5, 0.5, 0.5], None));
        assert!(bad(vec![1.2, -0.1, -0.1], None));
        assert!(bad(vec![f64::NAN, 0.5, 0.5], None));
        assert!(bad(vec![0.2, 0.3, 0.5], Some(1.5)));
    }

    /// Serve one canned HTTP response and hand back the request it received.
    async fn one_shot_server(body: &'static str) -> (String, tokio::task::JoinHandle<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                let read = socket.read(&mut buffer).await.unwrap();
                request.extend_from_slice(&buffer[..read]);
                let text = String::from_utf8_lossy(&request);
                if let Some((head, rest)) = text.split_once("\r\n\r\n") {
                    let length = head
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|value| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if rest.len() >= length {
                        break;
                    }
                }
            }
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            String::from_utf8(request).unwrap()
        });
        (format!("http://{address}/v1/decisions"), handle)
    }

    fn client(endpoint: String) -> DecisionClient {
        DecisionClient::new(DeciderConfig {
            enabled: true,
            id: "test".to_owned(),
            endpoint,
            location: "local".to_owned(),
            min_confidence: 0.6,
            max_state_chars: 1000,
            timeout_sec: 5,
        })
        .unwrap()
    }

    #[tokio::test]
    async fn decide_sends_a_choice_and_maps_the_answer() {
        let (endpoint, server) = one_shot_server(
            r#"{"distribution":[0.05,0.9,0.05],"decision":{"choice":"ignored"},"confidence":0.9}"#,
        )
        .await;
        let outcome = client(endpoint)
            .decide(
                "Action states: ...",
                &options(&["logic_rules", "logic_proof"]),
            )
            .await
            .unwrap();
        assert_eq!(outcome.action, "logic_proof");
        assert!((outcome.confidence - 0.9).abs() < 1e-9);
        let request = server.await.unwrap();
        let body: serde_json::Value =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body["kind"], "choice");
        assert_eq!(
            body["options"][2],
            "complete: finish the run and report the results"
        );
        assert_eq!(body["state"], "Action states: ...");
    }

    #[tokio::test]
    async fn unreachable_decider_is_an_error_not_a_decision() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/v1/decisions", listener.local_addr().unwrap());
        drop(listener);
        assert!(
            client(endpoint)
                .decide("s", &options(&["logic_rules"]))
                .await
                .is_err()
        );
    }

    #[test]
    fn state_is_bounded_and_keeps_action_states_first() {
        let states = vec![
            state("table_analysis", ActionStatus::Succeeded),
            state("logic_rules", ActionStatus::Pending),
        ];
        let long_result = "x ".repeat(500);
        let steps = [StepSummary {
            action: "table_analysis",
            ok: true,
            result: &long_result,
        }];
        let instruction = "指示".repeat(5000);
        let built = build_state(&instruction, &states, &steps, 600);
        assert!(built.chars().count() <= 600);
        assert!(built.starts_with("Action states:\n- table_analysis: succeeded"));
        assert!(built.contains("- logic_rules: pending"));
        assert!(built.contains("table_analysis ok: x x"));
        assert!(built.contains("Instruction:\n指示"));
    }
}
