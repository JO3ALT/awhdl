//! Trusted adapter that asks a human through HumanPort in approver mode.
//! The human sees the canonical action; the display text is only a title.
use crate::approval::{ApprovalDecision, ApprovalRequest};
use crate::config::{ApprovalAdapterConfig, McpServerConfig, ProjectConfig};
use crate::mcp::McpSession;
use anyhow::{Context, Result, bail, ensure};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::{Value, json};
use std::time::Duration;

/// A human decision and the action hash the approval device echoed back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanAnswer {
    pub decision: ApprovalDecision,
    pub echoed_action_hash: String,
}

#[async_trait(?Send)]
pub trait Approver {
    async fn ask(&self, request: &ApprovalRequest) -> Result<HumanAnswer>;
}

pub struct HumanPortApprover {
    config: ProjectConfig,
    server: McpServerConfig,
    poll: Duration,
}

impl HumanPortApprover {
    pub fn new(config: &ProjectConfig, settings: &ApprovalAdapterConfig) -> Result<Self> {
        let server = config
            .mcp
            .servers
            .get(&settings.server)
            .with_context(|| format!("approval server {} is not configured", settings.server))?
            .clone();
        Ok(Self {
            config: config.clone(),
            server,
            poll: Duration::from_millis(settings.poll_ms),
        })
    }
}

/// The task shown to the human: the canonical action in full, never only prose.
pub fn approval_task(request: &ApprovalRequest) -> Result<Value> {
    Ok(json!({
        "kind": "choice",
        "title": format!("承認依頼: {}", request.display),
        "prompt": "次の操作を実行してよいか判断してください。承認対象は下の正規化 action です（表題は参考表示）。",
        "detail": format!(
            "{}\n\naction_hash: {}\nexpires_at: {}",
            serde_json::to_string_pretty(&request.canonical_action)?,
            request.action_hash,
            request.expires_at.to_rfc3339()
        ),
        "choices": [
            {"value": "approve", "label": "承認する"},
            {"value": "deny", "label": "拒否する"}
        ],
        "approval_id": request.approval_id,
        "action_hash": request.action_hash,
    }))
}

/// Interpret an answered HumanPort task.
pub fn parse_answer(task: &Value) -> Result<HumanAnswer> {
    ensure!(
        task.get("status").and_then(Value::as_str) == Some("answered"),
        "approval task was not answered"
    );
    let decision = match task.pointer("/values/decision").and_then(Value::as_str) {
        Some("approve") => ApprovalDecision::Grant,
        Some("deny") => ApprovalDecision::Deny,
        other => bail!("unexpected approval answer {other:?}"),
    };
    let echoed_action_hash = task
        .get("action_hash")
        .and_then(Value::as_str)
        .context("approval answer does not echo the action hash")?
        .to_owned();
    Ok(HumanAnswer {
        decision,
        echoed_action_hash,
    })
}

#[async_trait(?Send)]
impl Approver for HumanPortApprover {
    async fn ask(&self, request: &ApprovalRequest) -> Result<HumanAnswer> {
        let mut session = McpSession::start(&self.config, &self.server).await?;
        let outcome = async {
            // Refuse a device that lets an MCP client answer its own request.
            let tools = session.tools().await?;
            ensure!(
                !tools
                    .iter()
                    .any(|tool| tool == "human.answer" || tool == "human.cancel"),
                "approval server exposes answering over MCP; run HumanPort in approver mode"
            );
            let task = session
                .call("human.request", json!({"task": approval_task(request)?}))
                .await?;
            let task_id = task
                .get("task_id")
                .and_then(Value::as_str)
                .context("HumanPort returned no task_id")?
                .to_owned();
            loop {
                ensure!(Utc::now() <= request.expires_at, "approval request expired");
                match session
                    .call("human.await", json!({"task_id": task_id}))
                    .await
                {
                    Ok(task) => return parse_answer(&task),
                    Err(error) if format!("{error:#}").contains("WAIT_TIMEOUT") => {
                        tokio::time::sleep(self.poll).await;
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        .await;
        session.close().await;
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn request() -> ApprovalRequest {
        ApprovalRequest {
            approval_id: Uuid::new_v4(),
            action_hash: "a".repeat(64),
            expires_at: Utc::now() + chrono::Duration::seconds(30),
            canonical_action: json!({"action": "push", "parameters": {"branch": "main"}}),
            display: "push (x) on git:push".to_owned(),
        }
    }

    #[test]
    fn task_carries_the_canonical_action_and_answers_are_strict() {
        let request = request();
        let task = approval_task(&request).unwrap();
        let detail = task["detail"].as_str().unwrap();
        assert!(detail.contains("\"branch\": \"main\"") && detail.contains(&request.action_hash));
        assert_eq!(task["action_hash"], json!(request.action_hash));
        let answered = |decision: &str| json!({"status": "answered", "values": {"decision": decision}, "action_hash": "h"});
        assert_eq!(
            parse_answer(&answered("approve")).unwrap().decision,
            ApprovalDecision::Grant
        );
        assert_eq!(
            parse_answer(&answered("deny")).unwrap().decision,
            ApprovalDecision::Deny
        );
        assert!(parse_answer(&answered("yes")).is_err());
        assert!(parse_answer(
            &json!({"status": "cancelled", "values": {"decision": "approve"}, "action_hash": "h"})
        )
        .is_err());
        assert!(
            parse_answer(&json!({"status": "answered", "values": {"decision": "approve"}}))
                .is_err()
        );
    }

    /// Runs the real HumanPort script in approver mode (headless: the GUI thread
    /// is not needed for the MCP side) through a stub that stubs out Tk.
    #[tokio::test]
    async fn humanport_approver_mode_hides_answering_and_waits() {
        // HumanPort is a sibling project; skip where only this repository exists.
        let Some(script) = crate::test_support::deployment_root()
            .map(|root| root.join("HumanPort/humanport.py"))
            .filter(|script| script.is_file())
        else {
            eprintln!("skipped: HumanPort is not present next to this repository");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let launcher = dir.path().join("humanport-headless");
        // Import the module (MCP functions only) and serve stdio without Tk.
        std::fs::write(
            &launcher,
            format!(
                "#!/usr/bin/env bash\nexport HUMANPORT_MODE=approver\nexec python3 -u -c 'import importlib.util,sys; s=importlib.util.spec_from_file_location(\"hp\", sys.argv[1]); m=importlib.util.module_from_spec(s); s.loader.exec_module(m); m.mcp()' \"{}\"\n",
                script.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let config = ProjectConfig::load(crate::test_support::project_root()).unwrap();
        let server = McpServerConfig {
            enabled: true,
            transport: "stdio".to_owned(),
            command: launcher.to_string_lossy().into_owned(),
        };
        let mut session = McpSession::start(&config, &server).await.unwrap();
        let tools = session.tools().await.unwrap();
        assert!(tools.contains(&"human.request".to_owned()));
        assert!(!tools.contains(&"human.answer".to_owned()));
        let task = session
            .call(
                "human.request",
                json!({"task": approval_task(&request()).unwrap()}),
            )
            .await
            .unwrap();
        let task_id = task["task_id"].as_str().unwrap();
        let answered = session
            .call(
                "human.answer",
                json!({"task_id": task_id, "values": {"decision": "approve"}}),
            )
            .await;
        assert!(answered.is_err(), "an MCP client must not answer");
        let pending = session
            .call("human.await", json!({"task_id": task_id}))
            .await;
        assert!(format!("{:#}", pending.unwrap_err()).contains("WAIT_TIMEOUT"));
        session.close().await;
    }
}
