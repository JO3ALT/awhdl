use crate::approval::{ApprovalDecision, ApprovalRequest, ApprovalScope};
use crate::budget::BudgetUsage;
use crate::effect::{EffectDecision, EffectRequest, Reconciliation};
use crate::execution::{ExecutionState, ResultEnvelope};
use anyhow::{Context, Result};
use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::{env, ffi::OsString};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct AuditEvent<'a> {
    pub timestamp: DateTime<Utc>,
    pub run_id: Uuid,
    pub event: &'a str,
    pub data: Value,
    pub budget: &'a BudgetUsage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunState {
    pub run_id: Uuid,
    pub phase: String,
    pub iteration: u32,
    pub last_action: Option<String>,
    pub budget: BudgetUsage,
    pub execution: ExecutionState,
}

pub struct RunStore {
    execution: ExecutionState,
    checkpoint_failed: bool,
    run_id: Uuid,
    dir: PathBuf,
    events: File,
    progress: Option<File>,
    activity: Option<File>,
}

impl RunStore {
    pub fn create(project_root: &Path, run_root: &str, prompt: &str) -> Result<Self> {
        let run_id = Uuid::new_v4();
        let root = if Path::new(run_root).is_absolute() {
            PathBuf::from(run_root)
        } else {
            project_root.join(run_root)
        };
        let dir = root.join(run_id.to_string());
        fs::create_dir_all(dir.join("artifacts"))
            .with_context(|| format!("failed to create run directory: {}", dir.display()))?;
        fs::write(dir.join("prompt.md"), prompt)?;
        let events = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(dir.join("events.jsonl"))?;
        let progress = env::var_os("AICONDUCTOR_PROGRESS_LOG")
            .filter(|value| !value.is_empty())
            .map(open_append_log)
            .transpose()?;
        let activity = env::var_os("AICONDUCTOR_ACTIVITY_LOG")
            .filter(|value| !value.is_empty())
            .map(open_append_log)
            .transpose()?;
        Ok(Self {
            execution: ExecutionState::new(run_id),
            checkpoint_failed: false,
            run_id,
            dir,
            events,
            progress,
            activity,
        })
    }

    /// Reopen an existing run from its authoritative checkpoint. The caller must
    /// hold exclusive ownership of this run directory; no lock broker exists yet.
    pub fn open(dir: &Path) -> Result<Self> {
        let run_id: Uuid = dir
            .file_name()
            .and_then(|name| name.to_str())
            .context("run directory has no UUID name")?
            .parse()?;
        let execution = ExecutionState::restore(&fs::read(dir.join("execution.json"))?)?;
        anyhow::ensure!(
            execution.run_id() == run_id,
            "checkpoint run identity mismatch"
        );
        let state_path = dir.join("state.json");
        if state_path.exists() {
            let state: Value = serde_json::from_slice(&fs::read(state_path)?)?;
            let expected_run_id = run_id.to_string();
            anyhow::ensure!(
                state.get("run_id").and_then(Value::as_str) == Some(expected_run_id.as_str()),
                "state run identity mismatch"
            );
        }
        // Restore converts STARTED to UNCERTAIN. Persist that before another call.
        atomic_json(dir, "execution.json", &execution)?;
        let events = OpenOptions::new()
            .append(true)
            .open(dir.join("events.jsonl"))?;
        let progress = env::var_os("AICONDUCTOR_PROGRESS_LOG")
            .filter(|value| !value.is_empty())
            .map(open_append_log)
            .transpose()?;
        let activity = env::var_os("AICONDUCTOR_ACTIVITY_LOG")
            .filter(|value| !value.is_empty())
            .map(open_append_log)
            .transpose()?;
        Ok(Self {
            execution,
            checkpoint_failed: false,
            run_id,
            dir: dir.to_path_buf(),
            events,
            progress,
            activity,
        })
    }

    pub fn execution(&self) -> &ExecutionState {
        &self.execution
    }

    /// Persist identity before polling the device future; bind completion locally.
    pub async fn invoke<T: Serialize>(
        &mut self,
        operation: &str,
        budget: &BudgetUsage,
        call: impl std::future::Future<Output = Result<T>>,
    ) -> Result<ResultEnvelope<T>> {
        let identity = self.update_execution(|state| state.begin(operation, None))?;
        self.event(
            "invocation_started",
            serde_json::json!({
                "operation": operation, "identity": identity,
            }),
            budget,
        )?;
        let outcome = call.await.and_then(|payload| {
            let encoded = serde_json::to_value(&payload)?;
            Ok((payload, encoded))
        });
        self.update_execution(|state| {
            state.finish_result(
                &identity,
                outcome.as_ref().ok().map(|(_, encoded)| encoded.clone()),
            )
        })?;
        self.event(
            "invocation_finished",
            serde_json::json!({
                "operation": operation, "identity": identity, "ok": outcome.is_ok(),
            }),
            budget,
        )?;
        outcome.map(|(payload, _)| ResultEnvelope { identity, payload })
    }

    /// Write effects are journaled before dispatch. A transport/semantic error is
    /// UNCERTAIN for remote writes because the provider might already have committed
    /// its action; a local write error is FAILED and may be retried.
    pub async fn invoke_effect<T, F, Fut>(
        &mut self,
        request: EffectRequest,
        budget: &BudgetUsage,
        call: F,
    ) -> Result<T>
    where
        T: Serialize + DeserializeOwned,
        F: FnOnce(String) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let decision = self.update_execution(|state| state.reserve_effect(&request))?;
        let (effect_id, key) = match decision {
            EffectDecision::Cached { effect_id, result } => {
                let effect = self.execution.effect(effect_id).expect("cached effect");
                let record = self
                    .execution
                    .invocation(effect.invocation_id.expect("cached invocation"))
                    .expect("cached record");
                let identity = record.identity.clone();
                let payload = serde_json::from_value(result)?;
                self.event(
                    "effect_reused",
                    serde_json::json!({"effect_id": effect_id,
                    "identity": identity, "action": request.action()}),
                    budget,
                )?;
                return Ok(payload);
            }
            EffectDecision::Dispatch {
                effect_id,
                idempotency_key,
            } => (effect_id, idempotency_key),
        };
        let identity = self.update_execution(|state| {
            let identity = state.begin(request.operation(), None)?;
            state.start_effect(effect_id, &identity)?;
            Ok(identity)
        })?;
        let approval_id = self.execution.approval_consumed_by(identity.invocation_id);
        self.event(
            "effect_started",
            serde_json::json!({
                "effect_id": effect_id, "identity": identity, "action": request.action(),
                "resource": request.resource(), "class": request.class(),
                "payload_hash": request.payload_hash(), "approval_id": approval_id,
            }),
            budget,
        )?;
        let outcome = call(key).await.and_then(|payload| {
            let encoded = serde_json::to_value(&payload)?;
            Ok((payload, encoded))
        });
        self.update_execution(|state| {
            state.finish_effect(
                effect_id,
                &identity,
                outcome.as_ref().ok().map(|(_, encoded)| encoded.clone()),
            )
        })?;
        let state = self
            .execution
            .effect(effect_id)
            .expect("finished effect")
            .state;
        self.event(
            "effect_finished",
            serde_json::json!({
                "effect_id": effect_id, "identity": identity, "state": state,
            }),
            budget,
        )?;
        outcome.map(|(payload, _)| payload)
    }

    /// Reserve without dispatch, so an approval can be requested for the Effect.
    pub fn reserve_effect(&mut self, request: &EffectRequest) -> Result<EffectDecision> {
        self.update_execution(|state| state.reserve_effect(request))
    }

    /// The returned request goes to a Human device; audit keeps only its hash.
    pub fn request_approval(
        &mut self,
        effect_id: Uuid,
        parameters: &Value,
        ttl: chrono::Duration,
        budget: &BudgetUsage,
    ) -> Result<ApprovalRequest> {
        let request = self.update_execution(|state| {
            state.request_approval(
                effect_id,
                parameters,
                ApprovalScope::SingleAction,
                Utc::now(),
                ttl,
            )
        })?;
        self.event(
            "approval_requested",
            serde_json::json!({
                "approval_id": request.approval_id, "effect_id": effect_id,
                "action_hash": request.action_hash, "expires_at": request.expires_at,
            }),
            budget,
        )?;
        Ok(request)
    }

    /// Trusted approval adapters only; the planner has no path to this call.
    pub fn decide_approval(
        &mut self,
        approval_id: Uuid,
        action_hash: &str,
        decision: ApprovalDecision,
        budget: &BudgetUsage,
    ) -> Result<()> {
        self.update_execution(|state| {
            state.decide_approval(approval_id, action_hash, decision, Utc::now())
        })?;
        let state = self.execution.approval(approval_id).expect("decided").state;
        self.event(
            "approval_decided",
            serde_json::json!({"approval_id": approval_id, "state": state}),
            budget,
        )
    }

    /// Trusted reconciliation must be based on evidence from the provider.
    pub fn reconcile_effect(
        &mut self,
        effect_id: Uuid,
        outcome: Reconciliation,
        budget: &BudgetUsage,
    ) -> Result<()> {
        self.update_execution(|state| state.reconcile_effect(effect_id, outcome))?;
        let effect = self.execution.effect(effect_id).expect("reconciled effect");
        self.event(
            "effect_reconciled",
            serde_json::json!({
                "effect_id": effect_id, "state": effect.state,
            }),
            budget,
        )
    }

    /// Commit a clone before exposing a transition. An uncertain write poisons this
    /// store so callers cannot reuse an older in-memory consumption snapshot.
    fn update_execution<T>(
        &mut self,
        update: impl FnOnce(&mut ExecutionState) -> Result<T>,
    ) -> Result<T> {
        anyhow::ensure!(
            !self.checkpoint_failed,
            "execution checkpoint failed; reopen from durable state"
        );
        let mut next = self.execution.clone();
        let result = update(&mut next)?;
        if let Err(error) = atomic_json(&self.dir, "execution.json", &next) {
            self.checkpoint_failed = true;
            return Err(error);
        }
        self.execution = next;
        Ok(result)
    }

    pub fn assign_value(&mut self, name: &str, payload: Value) -> Result<Option<Uuid>> {
        self.update_execution(|state| state.assign(name, payload))
    }

    pub fn publish_event(
        &mut self,
        topic: &str,
        payload: Value,
        cause: Option<Uuid>,
    ) -> Result<Uuid> {
        self.update_execution(|state| state.publish(topic, payload, cause))
    }

    /// At-most-once delivery: persist consumption BEFORE returning the payload.
    /// A crash after this commit can lose handling; effect recovery is Phase 3.
    pub fn consume_event(
        &mut self,
        event_id: Uuid,
    ) -> Result<Option<crate::dataflow::Event<Value>>> {
        self.update_execution(|state| state.consume(event_id))
    }

    pub fn id(&self) -> Uuid {
        self.run_id
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn event(&mut self, event: &str, data: Value, budget: &BudgetUsage) -> Result<()> {
        let record = AuditEvent {
            timestamp: Utc::now(),
            run_id: self.run_id,
            event,
            data,
            budget,
        };
        let encoded = serde_json::to_vec(&record)?;
        self.events.write_all(&encoded)?;
        self.events.write_all(b"\n")?;
        self.events.flush()?;
        if let Some(progress) = &mut self.progress {
            progress.write_all(&encoded)?;
            progress.write_all(b"\n")?;
            progress.flush()?;
        }
        if let Some(activity) = &mut self.activity {
            activity.write_all(format_activity_event(&record).as_bytes())?;
            activity.flush()?;
        }
        Ok(())
    }

    pub fn checkpoint(&self, state: &RunState) -> Result<()> {
        anyhow::ensure!(
            state.run_id == self.run_id && state.execution.run_id() == self.run_id,
            "checkpoint run identity mismatch"
        );
        atomic_json(&self.dir, "state.json", state)
    }

    pub fn write_result(&self, result: &str) -> Result<()> {
        fs::write(self.dir.join("result.md"), result)?;
        Ok(())
    }
}

fn open_append_log(path: OsString) -> Result<File> {
    let path = PathBuf::from(path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("failed to open activity log: {}", path.display()))
}

fn format_activity_event(record: &AuditEvent<'_>) -> String {
    let detail = ["action", "profile", "tool", "error"]
        .iter()
        .find_map(|key| record.data.get(key).and_then(Value::as_str))
        .map(|value| truncate(value, 160));
    let detail = detail
        .map(|value| format!(" | {value}"))
        .unwrap_or_default();
    format!(
        "[{}] progress {} (iteration={} model={} mcp={} switch={}){}\n",
        record
            .timestamp
            .with_timezone(&Local)
            .format("%Y-%m-%dT%H:%M:%S%:z"),
        record.event,
        record.budget.iterations,
        record.budget.model_calls,
        record.budget.mcp_calls,
        record.budget.model_switches,
        detail,
    )
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

fn atomic_json(dir: &Path, name: &str, value: &impl Serialize) -> Result<()> {
    let temporary = dir.join(format!(".{name}.tmp"));
    let mut file = File::create(&temporary)?;
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.sync_all()?;
    fs::rename(temporary, dir.join(name))?;
    File::open(dir)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod identity_tests {
    use super::*;

    #[tokio::test]
    async fn device_identity_is_durable_before_dispatch_and_completion_is_bound() {
        let root = tempfile::tempdir().unwrap();
        let mut store = RunStore::create(root.path(), "runs", "test").unwrap();
        let path = store.dir().join("execution.json");
        let result = store
            .invoke("mock", &BudgetUsage::default(), async {
                let encoded = fs::read(&path)?;
                let mut restored = ExecutionState::restore(&encoded)?;
                assert!(restored.begin("mock", None).is_err());
                Ok("response")
            })
            .await
            .unwrap();
        assert_eq!(result.payload, "response");
        let restored = ExecutionState::restore(&fs::read(&path).unwrap()).unwrap();
        assert!(
            restored
                .barrier_ready(&["mock"], std::slice::from_ref(&result.identity))
                .unwrap()
        );
        let audit = fs::read_to_string(store.dir().join("events.jsonl")).unwrap();
        assert!(audit.contains(&result.identity.invocation_id.to_string()));
        assert!(!audit.contains("response"));
    }

    #[tokio::test]
    async fn failed_call_keeps_identity_and_retry_advances_attempt() {
        let root = tempfile::tempdir().unwrap();
        let mut store = RunStore::create(root.path(), "runs", "test").unwrap();
        assert!(
            store
                .invoke::<()>("mock", &BudgetUsage::default(), async {
                    anyhow::bail!("transport failed")
                })
                .await
                .is_err()
        );
        let retry = store
            .invoke("mock", &BudgetUsage::default(), async { Ok(()) })
            .await
            .unwrap();
        assert_eq!(retry.identity.attempt, 2);
    }
}

#[cfg(test)]
mod dataflow_tests {
    use super::*;
    use crate::dataflow::EventKind;
    use crate::execution::InvocationStatus;
    use serde_json::json;

    #[test]
    fn event_is_durably_consumed_before_delivery_and_values_survive() {
        let root = tempfile::tempdir().unwrap();
        let mut store = RunStore::create(root.path(), "runs", "test").unwrap();
        let change = store
            .assign_value("latest", json!("private value"))
            .unwrap()
            .unwrap();
        let occurrence = store
            .publish_event("timer", json!(7), Some(change))
            .unwrap();
        assert!(store.consume_event(change).unwrap().is_some());
        let encoded = fs::read(store.dir().join("execution.json")).unwrap();
        let mut restored = ExecutionState::restore(&encoded).unwrap();
        assert!(restored.consume(change).unwrap().is_none());
        assert_eq!(restored.next_event().unwrap().event_id, occurrence);
        assert_eq!(
            restored.value("latest").unwrap().payload,
            json!("private value")
        );
        let audit = fs::read_to_string(store.dir().join("events.jsonl")).unwrap();
        assert!(!audit.contains("private value"));
    }

    #[tokio::test]
    async fn completed_result_and_notification_are_in_the_same_checkpoint() {
        let root = tempfile::tempdir().unwrap();
        let mut store = RunStore::create(root.path(), "runs", "test").unwrap();
        let result = store
            .invoke("mock", &BudgetUsage::default(), async {
                Ok(json!({"restricted": "private output"}))
            })
            .await
            .unwrap();
        let restored =
            ExecutionState::restore(&fs::read(store.dir().join("execution.json")).unwrap())
                .unwrap();
        let record = restored.invocation(result.identity.invocation_id).unwrap();
        assert_eq!(record.result.as_ref().unwrap().payload, result.payload);
        assert_eq!(
            restored.next_event().unwrap().kind,
            EventKind::InvocationTerminated {
                invocation_id: result.identity.invocation_id,
                status: InvocationStatus::Completed,
            }
        );
        let audit = fs::read_to_string(store.dir().join("events.jsonl")).unwrap();
        assert!(!audit.contains("private output"));
    }

    #[tokio::test]
    async fn checkpoint_failure_does_not_deliver_or_allow_further_dispatch() {
        let root = tempfile::tempdir().unwrap();
        let mut store = RunStore::create(root.path(), "runs", "test").unwrap();
        let id = store.publish_event("request", json!(1), None).unwrap();
        let obstruction = store.dir().join(".execution.json.tmp");
        fs::create_dir(&obstruction).unwrap();
        assert!(store.consume_event(id).is_err());
        assert_eq!(store.execution().next_event().unwrap().event_id, id);
        let restored =
            ExecutionState::restore(&fs::read(store.dir().join("execution.json")).unwrap())
                .unwrap();
        assert_eq!(restored.next_event().unwrap().event_id, id);
        fs::remove_dir(obstruction).unwrap();
        assert!(store.consume_event(id).is_err());
        let mut polled = false;
        let outcome = store
            .invoke("mock", &BudgetUsage::default(), async {
                polled = true;
                Ok(())
            })
            .await;
        assert!(outcome.is_err());
        assert!(!polled);
    }
}

#[cfg(test)]
mod effect_tests {
    use super::*;
    use crate::effect::{EffectClass, EffectRequest, EffectState};
    use serde_json::json;

    fn request() -> EffectRequest {
        EffectRequest::new(
            "action:send",
            "send",
            "mail:send",
            &json!({"to":"a"}),
            EffectClass::ExternalWrite,
            Some("idempotency_key".to_owned()),
            false,
        )
        .unwrap()
        .with_approval(false)
        .unwrap()
    }

    #[tokio::test]
    async fn started_is_durable_before_provider_call_and_confirmed_result_is_reused() {
        let root = tempfile::tempdir().unwrap();
        let mut store = RunStore::create(root.path(), "runs", "test").unwrap();
        let checkpoint = store.dir().join("execution.json");
        let first = store
            .invoke_effect(request(), &BudgetUsage::default(), |key| async move {
                assert!(key.starts_with("awhdl:"));
                let restored = ExecutionState::restore(&fs::read(&checkpoint)?)?;
                let effect = restored.effects.values().next().unwrap();
                assert_eq!(effect.state, EffectState::Uncertain); // STARTED at crash is recovered as UNCERTAIN
                assert!(effect.idempotency_key == key);
                Ok(json!("provider result"))
            })
            .await
            .unwrap();
        let again = store
            .invoke_effect(request(), &BudgetUsage::default(), |_| async {
                panic!("confirmed effect must not be dispatched again");
                #[allow(unreachable_code)]
                Ok(json!("duplicate"))
            })
            .await
            .unwrap();
        assert_eq!(again, first);
        let audit = fs::read_to_string(store.dir().join("events.jsonl")).unwrap();
        assert!(!audit.contains("provider result"));
    }

    #[tokio::test]
    async fn timeout_blocks_redelivery_until_trusted_not_found_reconciliation() {
        let root = tempfile::tempdir().unwrap();
        let mut store = RunStore::create(root.path(), "runs", "test").unwrap();
        let mut key = String::new();
        let first = store
            .invoke_effect::<String, _, _>(request(), &BudgetUsage::default(), |idempotency| {
                key = idempotency;
                async { anyhow::bail!("provider timeout") }
            })
            .await;
        assert!(first.is_err());
        let effect_id = *store.execution.effects.keys().next().unwrap();
        assert_eq!(
            store.execution.effect(effect_id).unwrap().state,
            EffectState::Uncertain
        );
        assert!(
            store
                .invoke_effect(request(), &BudgetUsage::default(), |_| async {
                    panic!("uncertain effect must not be dispatched");
                    #[allow(unreachable_code)]
                    Ok(String::new())
                })
                .await
                .is_err()
        );
        store
            .reconcile_effect(effect_id, Reconciliation::NotFound, &BudgetUsage::default())
            .unwrap();
        let retry = store
            .invoke_effect(
                request(),
                &BudgetUsage::default(),
                |reused_key| async move {
                    assert_eq!(reused_key, key);
                    Ok("sent".to_owned())
                },
            )
            .await
            .unwrap();
        assert_eq!(retry, "sent");
        let effect = store.execution.effect(effect_id).unwrap();
        let invocation = store
            .execution
            .invocation(effect.invocation_id.unwrap())
            .unwrap();
        assert_eq!(invocation.identity.attempt, 2);
    }
}

#[cfg(test)]
mod restart_effect_tests {
    use super::*;
    use crate::effect::{EffectClass, EffectDecision, EffectRequest, EffectState};
    use serde_json::json;

    #[test]
    fn reopened_run_persists_uncertain_then_reconciles_without_redelivery() {
        let root = tempfile::tempdir().unwrap();
        let mut store = RunStore::create(root.path(), "runs", "test").unwrap();
        let request = EffectRequest::new(
            "action:send",
            "send",
            "mail:send",
            &json!({"to":"a"}),
            EffectClass::ExternalWrite,
            None,
            true,
        )
        .unwrap()
        .with_approval(false)
        .unwrap();
        let EffectDecision::Dispatch { effect_id, .. } = store
            .update_execution(|s| s.reserve_effect(&request))
            .unwrap()
        else {
            panic!("dispatch")
        };
        let identity = store
            .update_execution(|s| {
                let identity = s.begin(request.operation(), None)?;
                s.start_effect(effect_id, &identity)?;
                Ok(identity)
            })
            .unwrap();
        let run_dir = store.dir().to_path_buf();
        drop(store);
        let mut reopened = RunStore::open(&run_dir).unwrap();
        assert_eq!(
            reopened.execution.effect(effect_id).unwrap().state,
            EffectState::Uncertain
        );
        assert_eq!(
            ExecutionState::restore(&fs::read(run_dir.join("execution.json")).unwrap())
                .unwrap()
                .effect(effect_id)
                .unwrap()
                .state,
            EffectState::Uncertain
        );
        assert!(
            reopened
                .update_execution(|s| s.reserve_effect(&request))
                .is_err()
        );
        reopened
            .reconcile_effect(effect_id, Reconciliation::NotFound, &BudgetUsage::default())
            .unwrap();
        assert_eq!(
            reopened
                .execution
                .invocation(identity.invocation_id)
                .unwrap()
                .status,
            crate::execution::InvocationStatus::Failed
        );
        assert!(
            matches!(reopened.update_execution(|s| s.reserve_effect(&request)).unwrap(),
            EffectDecision::Dispatch { effect_id: same, .. } if same == effect_id)
        );
    }
}

#[cfg(test)]
mod approval_tests {
    use super::*;
    use crate::approval::{ApprovalDecision, ApprovalState};
    use crate::effect::{EffectClass, EffectDecision, EffectRequest, EffectState};
    use serde_json::json;

    #[tokio::test]
    async fn run_store_dispatches_only_with_a_persisted_single_use_approval() {
        let root = tempfile::tempdir().unwrap();
        let mut store = RunStore::create(root.path(), "runs", "test").unwrap();
        let budget = BudgetUsage::default();
        let parameters = json!({"repository": "lab/repo", "commit": "123abc"});
        let request = || {
            EffectRequest::new(
                "action:push",
                "push",
                "git:push",
                &parameters,
                EffectClass::ExternalWrite,
                None,
                true,
            )
            .unwrap()
        };
        let denied = store
            .invoke_effect::<String, _, _>(request(), &budget, |_| async {
                panic!("unapproved effect must not reach the provider")
            })
            .await;
        assert!(denied.is_err());
        let EffectDecision::Dispatch { effect_id, .. } = store.reserve_effect(&request()).unwrap()
        else {
            panic!("dispatch")
        };
        assert_eq!(
            store.execution.effect(effect_id).unwrap().state,
            EffectState::NotStarted
        );
        let approval = store
            .request_approval(
                effect_id,
                &parameters,
                chrono::Duration::minutes(5),
                &budget,
            )
            .unwrap();
        store
            .decide_approval(
                approval.approval_id,
                &approval.action_hash,
                ApprovalDecision::Grant,
                &budget,
            )
            .unwrap();
        let sent = store
            .invoke_effect(request(), &budget, |_| async { Ok("pushed".to_owned()) })
            .await
            .unwrap();
        assert_eq!(sent, "pushed");
        let dir = store.dir().to_path_buf();
        drop(store);
        let reopened = RunStore::open(&dir).unwrap();
        let persisted = reopened.execution.approval(approval.approval_id).unwrap();
        assert_eq!(persisted.state, ApprovalState::Consumed);
        let events = fs::read_to_string(dir.join("events.jsonl")).unwrap();
        assert!(events.contains("approval_requested") && events.contains("approval_decided"));
        assert!(
            !events.contains("123abc"),
            "audit must not carry raw parameters"
        );
    }
}
