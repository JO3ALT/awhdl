//! Trusted execution identity; device payloads never supply these fields.
use anyhow::{Result, bail, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub const EXECUTION_VERSION: u32 = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvocationIdentity {
    pub workflow_run_id: Uuid,
    pub invocation_id: Uuid,
    pub parent_invocation_id: Option<Uuid>,
    pub causation_id: Option<Uuid>,
    pub correlation_id: Uuid,
    pub generation: u64,
    pub attempt: u32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvocationStatus {
    Pending,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invocation<T> {
    pub identity: InvocationIdentity,
    pub operation: String,
    pub status: InvocationStatus,
    pub result: Option<InvocationResult<T>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvocationResult<T> {
    pub payload: T,
}

pub type InvocationRecord = Invocation<serde_json::Value>;

/// Versioned execution portion of IR/checkpoints, independent of source AST.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionState {
    version: u32,
    workflow_run_id: Uuid,
    correlation_id: Uuid,
    generation: u64,
    records: BTreeMap<Uuid, InvocationRecord>,
    pub(crate) dataflow: crate::dataflow::DataflowState,
    pub(crate) effects: BTreeMap<Uuid, crate::effect::Effect>,
    pub(crate) approvals: BTreeMap<Uuid, crate::approval::Approval>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResultEnvelope<T> {
    pub identity: InvocationIdentity,
    pub payload: T,
}

impl ExecutionState {
    pub fn new(workflow_run_id: Uuid) -> Self {
        Self {
            version: EXECUTION_VERSION,
            workflow_run_id,
            correlation_id: Uuid::new_v4(),
            generation: 0,
            records: BTreeMap::new(),
            dataflow: crate::dataflow::DataflowState::default(),
            effects: BTreeMap::new(),
            approvals: BTreeMap::new(),
        }
    }

    pub fn run_id(&self) -> Uuid {
        self.workflow_run_id
    }

    pub fn begin(&mut self, operation: &str, parent: Option<Uuid>) -> Result<InvocationIdentity> {
        self.begin_caused(operation, parent, None)
    }

    pub fn begin_caused(
        &mut self,
        operation: &str,
        parent: Option<Uuid>,
        causation_id: Option<Uuid>,
    ) -> Result<InvocationIdentity> {
        self.validate_cause(causation_id, &self.scope())?;
        ensure!(!operation.is_empty(), "empty invocation operation");
        if let Some(parent) = parent {
            let record = self
                .records
                .get(&parent)
                .ok_or_else(|| anyhow::anyhow!("unknown parent"))?;
            self.check_scope(&record.identity)?;
        }
        let previous = self
            .records
            .values()
            .filter(|record| {
                record.operation == operation && self.check_scope(&record.identity).is_ok()
            })
            .collect::<Vec<_>>();
        ensure!(
            previous
                .iter()
                .all(|r| r.status != InvocationStatus::Pending),
            "operation already pending"
        );
        let attempt = previous
            .iter()
            .map(|r| r.identity.attempt)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("attempt overflow"))?;
        let identity = InvocationIdentity {
            workflow_run_id: self.workflow_run_id,
            invocation_id: Uuid::new_v4(),
            parent_invocation_id: parent,
            causation_id,
            correlation_id: self.correlation_id,
            generation: self.generation,
            attempt,
            created_at: Utc::now(),
        };
        self.records.insert(
            identity.invocation_id,
            InvocationRecord {
                identity: identity.clone(),
                operation: operation.to_owned(),
                status: InvocationStatus::Pending,
                result: None,
            },
        );
        Ok(identity)
    }

    fn check_scope(&self, identity: &InvocationIdentity) -> Result<()> {
        ensure!(
            identity.workflow_run_id == self.workflow_run_id
                && identity.correlation_id == self.correlation_id
                && identity.generation == self.generation,
            "stale or foreign invocation scope"
        );
        Ok(())
    }

    pub fn finish(&mut self, identity: &InvocationIdentity, succeeded: bool) -> Result<()> {
        self.finish_result(
            identity,
            if succeeded {
                Some(serde_json::Value::Null)
            } else {
                None
            },
        )
    }

    pub fn finish_result(
        &mut self,
        identity: &InvocationIdentity,
        result: Option<serde_json::Value>,
    ) -> Result<()> {
        let status = if result.is_some() {
            InvocationStatus::Completed
        } else {
            InvocationStatus::Failed
        };
        self.terminate(identity, status, result)
    }

    pub fn cancel(&mut self, identity: &InvocationIdentity) -> Result<()> {
        self.terminate(identity, InvocationStatus::Cancelled, None)
    }

    fn terminate(
        &mut self,
        identity: &InvocationIdentity,
        status: InvocationStatus,
        result: Option<serde_json::Value>,
    ) -> Result<()> {
        self.check_scope(identity)?;
        let record = self
            .records
            .get(&identity.invocation_id)
            .ok_or_else(|| anyhow::anyhow!("unknown invocation"))?;
        ensure!(&record.identity == identity, "invocation identity mismatch");
        ensure!(
            record.status == InvocationStatus::Pending,
            "duplicate invocation completion"
        );
        self.emit_event(
            crate::dataflow::EventKind::InvocationTerminated {
                invocation_id: identity.invocation_id,
                status,
            },
            serde_json::Value::Null,
            identity.causation_id,
        )?;
        let record = self
            .records
            .get_mut(&identity.invocation_id)
            .expect("validated invocation");
        record.status = status;
        record.result = result.map(|payload| InvocationResult { payload });
        Ok(())
    }

    pub fn invocation(&self, id: Uuid) -> Option<&InvocationRecord> {
        self.records.get(&id)
    }

    pub(crate) fn invocations(&self) -> impl Iterator<Item = &InvocationRecord> {
        self.records.values()
    }

    pub fn scope(&self) -> crate::dataflow::Scope {
        crate::dataflow::Scope {
            workflow_run_id: self.workflow_run_id,
            correlation_id: self.correlation_id,
            generation: self.generation,
        }
    }

    /// A logical revision invalidates all earlier results, including pending calls.
    pub fn advance_generation(&mut self) -> Result<()> {
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("generation overflow"))?;
        Ok(())
    }

    /// Required operation names represent branch slots, not merely existing values.
    pub fn barrier_ready(&self, required: &[&str], results: &[InvocationIdentity]) -> Result<bool> {
        ensure!(!required.is_empty(), "empty barrier");
        ensure!(
            required.iter().collect::<BTreeSet<_>>().len() == required.len(),
            "duplicate required branch"
        );
        let mut seen = BTreeMap::new();
        for identity in results {
            self.check_scope(identity)?;
            let record = self
                .records
                .get(&identity.invocation_id)
                .ok_or_else(|| anyhow::anyhow!("unknown barrier invocation"))?;
            ensure!(&record.identity == identity, "barrier identity mismatch");
            ensure!(
                required.contains(&record.operation.as_str()),
                "unexpected barrier branch"
            );
            ensure!(
                seen.insert(record.operation.as_str(), record.status)
                    .is_none(),
                "duplicate barrier branch"
            );
            // A successful older attempt cannot satisfy a slot after a newer attempt starts.
            ensure!(
                !self
                    .records
                    .values()
                    .any(|other| other.operation == record.operation
                        && self.check_scope(&other.identity).is_ok()
                        && other.identity.attempt > identity.attempt),
                "superseded barrier result"
            );
        }
        Ok(required
            .iter()
            .all(|name| seen.get(name) == Some(&InvocationStatus::Completed)))
    }

    /// Restore validated execution/dataflow state without replaying events or retrying calls.
    pub fn restore(encoded: &[u8]) -> Result<Self> {
        let mut state: Self = serde_json::from_slice(encoded)?;
        ensure!(
            state.version == EXECUTION_VERSION,
            "unsupported execution version"
        );
        let mut attempts = BTreeSet::new();
        for (id, record) in &state.records {
            ensure!(!record.operation.is_empty(), "empty checkpoint operation");
            ensure!(
                attempts.insert((
                    record.identity.generation,
                    &record.operation,
                    record.identity.attempt
                )),
                "duplicate checkpoint attempt"
            );
            ensure!(
                (record.status == InvocationStatus::Completed) == record.result.is_some(),
                "invalid invocation result state"
            );
            let identity = &record.identity;
            ensure!(
                *id == identity.invocation_id
                    && identity.workflow_run_id == state.workflow_run_id
                    && identity.correlation_id == state.correlation_id
                    && identity.generation <= state.generation
                    && identity.attempt > 0,
                "invalid checkpoint identity"
            );
            let mut ancestors = BTreeSet::from([*id]);
            let mut next = identity.parent_invocation_id;
            while let Some(parent) = next {
                ensure!(ancestors.insert(parent), "cyclic checkpoint parent");
                next = state
                    .records
                    .get(&parent)
                    .ok_or_else(|| anyhow::anyhow!("missing checkpoint parent"))?
                    .identity
                    .parent_invocation_id;
            }
            if let Some(parent) = identity.parent_invocation_id {
                let Some(parent_record) = state.records.get(&parent) else {
                    bail!("missing checkpoint parent")
                };
                ensure!(
                    parent != *id && parent_record.identity.generation == identity.generation,
                    "invalid checkpoint parent"
                );
            }
        }
        state.validate_dataflow()?;
        state.validate_effects()?;
        state.validate_approvals()?;
        // A call in flight at crash time may have succeeded remotely.
        for effect in state.effects.values_mut() {
            if effect.state == crate::effect::EffectState::Started {
                effect.state = crate::effect::EffectState::Uncertain;
            }
        }
        Ok(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn delayed_device_completion_is_rejected_after_revision() {
        let mut state = ExecutionState::new(Uuid::new_v4());
        let old = state.begin("coder", None).unwrap();
        let (release, waiting) = tokio::sync::oneshot::channel::<()>();
        let device = tokio::spawn(async move {
            waiting.await.unwrap();
            ResultEnvelope {
                identity: old,
                payload: "old candidate",
            }
        });
        state.advance_generation().unwrap();
        let current = state.begin("coder", None).unwrap();
        state.finish(&current, true).unwrap();
        release.send(()).unwrap();
        let late = device.await.unwrap();
        assert!(state.finish(&late.identity, true).is_err());
        assert!(state.barrier_ready(&["coder"], &[current]).unwrap());
    }

    #[test]
    fn late_results_cannot_enter_a_new_generation() {
        let mut state = ExecutionState::new(Uuid::new_v4());
        let old = state.begin("coder", None).unwrap();
        state.advance_generation().unwrap();
        let new = state.begin("coder", None).unwrap();
        assert!(state.finish(&old, true).is_err());
        state.finish(&new, true).unwrap();
        assert_eq!(new.generation, old.generation + 1);
        assert_eq!(new.attempt, 1);
    }

    #[test]
    fn retry_has_new_identity_and_incremented_attempt() {
        let mut state = ExecutionState::new(Uuid::new_v4());
        let first = state.begin("coder", None).unwrap();
        assert!(state.begin("coder", None).is_err());
        state.finish(&first, false).unwrap();
        let retry = state.begin("coder", None).unwrap();
        assert_ne!(first.invocation_id, retry.invocation_id);
        assert_eq!(retry.attempt, 2);
        assert_eq!(retry.generation, first.generation);
        assert!(state.finish(&first, true).is_err());
    }

    #[test]
    fn parallel_barrier_requires_matching_scope_and_all_branches() {
        let mut state = ExecutionState::new(Uuid::new_v4());
        let parent = state.begin("validation", None).unwrap();
        let build = state.begin("build", Some(parent.invocation_id)).unwrap();
        let tests = state.begin("tests", Some(parent.invocation_id)).unwrap();
        assert_eq!(build.generation, tests.generation);
        assert_eq!(build.correlation_id, tests.correlation_id);
        state.finish(&build, true).unwrap();
        assert!(
            !state
                .barrier_ready(&["build", "tests"], &[build.clone(), tests.clone()])
                .unwrap()
        );
        state.finish(&tests, true).unwrap();
        assert!(
            state
                .barrier_ready(&["build", "tests"], &[build.clone(), tests.clone()])
                .unwrap()
        );
        state.advance_generation().unwrap();
        let fresh = state.begin("tests", None).unwrap();
        state.finish(&fresh, true).unwrap();
        assert!(
            state
                .barrier_ready(&["build", "tests"], &[build, fresh])
                .is_err()
        );
    }

    #[test]
    fn checkpoint_restores_pending_identity_without_reissuing() {
        let mut state = ExecutionState::new(Uuid::new_v4());
        let identity = state.begin("mcp", None).unwrap();
        let mut restored = ExecutionState::restore(&serde_json::to_vec(&state).unwrap()).unwrap();
        assert!(restored.begin("mcp", None).is_err());
        restored.finish(&identity, true).unwrap();
        assert!(restored.barrier_ready(&["mcp"], &[identity]).unwrap());
    }

    #[test]
    fn foreign_forged_duplicate_and_superseded_results_are_rejected() {
        let mut state = ExecutionState::new(Uuid::new_v4());
        let original = state.begin("work", None).unwrap();
        let mut forged = original.clone();
        forged.correlation_id = Uuid::new_v4();
        assert!(state.finish(&forged, true).is_err());
        forged = original.clone();
        forged.attempt += 1;
        assert!(state.finish(&forged, true).is_err());
        state.finish(&original, true).unwrap();
        assert!(state.finish(&original, true).is_err());
        state.begin("work", None).unwrap();
        assert!(state.barrier_ready(&["work"], &[original]).is_err());
        assert!(ExecutionState::restore(br#"{"version":0}"#).is_err());
    }
}
