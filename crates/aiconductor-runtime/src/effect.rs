//! Durable external effects. Model/provider output never chooses the effect identity.
use crate::dataflow::Scope;
use crate::execution::{ExecutionState, InvocationIdentity, InvocationResult, InvocationStatus};
use anyhow::{Result, bail, ensure};
use chrono::{DateTime, Utc};
use ring::digest::{SHA256, digest};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectClass {
    Pure,
    Read,
    LocalWrite,
    ExternalWrite,
    Destructive,
}

impl EffectClass {
    /// Severity order used when a narrower or overlapping grant is compared.
    pub fn rank(self) -> u8 {
        match self {
            Self::Pure => 0,
            Self::Read => 1,
            Self::LocalWrite => 2,
            Self::ExternalWrite => 3,
            Self::Destructive => 4,
        }
    }

    pub fn is_write(self) -> bool {
        matches!(
            self,
            Self::LocalWrite | Self::ExternalWrite | Self::Destructive
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectState {
    NotStarted,
    Started,
    Confirmed,
    Failed,
    Uncertain,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Effect {
    pub effect_id: Uuid,
    pub scope: Scope,
    pub operation: String,
    pub action: String,
    pub resource: String,
    pub payload_hash: String,
    pub class: EffectClass,
    /// Handle (`id#fingerprint`) of the capability that authorized this Effect.
    pub capability: String,
    pub idempotency_key: String,
    pub state: EffectState,
    /// Every dispatch of this Effect must consume its own granted approval.
    pub requires_approval: bool,
    pub invocation_id: Option<Uuid>,
    pub result: Option<InvocationResult<Json>>,
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    digest(&SHA256, bytes)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// SHA-256 over serde_json's encoding; maps are key-sorted (no `preserve_order`).
pub(crate) fn payload_hash(payload: &Json) -> Result<String> {
    Ok(sha256_hex(&serde_json::to_vec(payload)?))
}

#[derive(Debug, Clone)]
pub struct EffectRequest {
    operation: String,
    action: String,
    resource: String,
    payload_hash: String,
    class: EffectClass,
    requires_approval: bool,
    capability: String,
}

/// Placeholder for Effects created without a policy decision (API tests).
pub const UNSCOPED_CAPABILITY: &str = "unscoped";

impl EffectRequest {
    pub fn operation(&self) -> &str {
        &self.operation
    }
    pub fn action(&self) -> &str {
        &self.action
    }
    pub fn resource(&self) -> &str {
        &self.resource
    }
    pub fn payload_hash(&self) -> &str {
        &self.payload_hash
    }
    pub fn class(&self) -> EffectClass {
        self.class
    }
    pub fn requires_approval(&self) -> bool {
        self.requires_approval
    }
    pub fn capability(&self) -> &str {
        &self.capability
    }
    /// Bind the Effect to the capability handle returned by authorization.
    pub fn with_capability(mut self, handle: &str) -> Result<Self> {
        ensure!(!handle.is_empty(), "empty capability handle");
        self.capability = handle.to_owned();
        Ok(self)
    }
    /// External and destructive writes require approval unless a route opts out.
    /// A destructive write cannot opt out.
    pub fn with_approval(mut self, required: bool) -> Result<Self> {
        ensure!(
            required || self.class != EffectClass::Destructive,
            "destructive effect always requires approval"
        );
        self.requires_approval = required;
        Ok(self)
    }
    pub fn new(
        operation: &str,
        action: &str,
        resource: &str,
        payload: &Json,
        class: EffectClass,
        idempotency_argument: Option<String>,
        manual_reconciliation: bool,
    ) -> Result<Self> {
        ensure!(
            !operation.is_empty() && !action.is_empty() && !resource.is_empty(),
            "empty effect descriptor"
        );
        ensure!(class.is_write(), "read-only call is not an effect");
        ensure!(
            idempotency_argument
                .as_ref()
                .is_none_or(|name| !name.is_empty()),
            "empty idempotency argument"
        );
        if matches!(class, EffectClass::ExternalWrite | EffectClass::Destructive) {
            ensure!(
                idempotency_argument.is_some() || manual_reconciliation,
                "external/destructive write needs provider idempotency or explicit reconciliation policy"
            );
        }
        Ok(Self {
            operation: operation.to_owned(),
            action: action.to_owned(),
            resource: resource.to_owned(),
            payload_hash: payload_hash(payload)?,
            class,
            requires_approval: matches!(
                class,
                EffectClass::ExternalWrite | EffectClass::Destructive
            ),
            capability: UNSCOPED_CAPABILITY.to_owned(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectDecision {
    Dispatch {
        effect_id: Uuid,
        idempotency_key: String,
    },
    Cached {
        effect_id: Uuid,
        result: Json,
    },
}

#[derive(Debug, Clone)]
pub enum Reconciliation {
    Confirmed(Json),
    NotFound,
    StillUncertain,
}

impl ExecutionState {
    pub fn effect(&self, id: Uuid) -> Option<&Effect> {
        self.effects.get(&id)
    }

    pub fn effects(&self) -> impl Iterator<Item = &Effect> {
        self.effects.values()
    }

    /// Reserve an exact action instance. A changed payload or destination is a new
    /// instance with its own key, never a reuse of an earlier instance's key.
    pub fn reserve_effect(&mut self, request: &EffectRequest) -> Result<EffectDecision> {
        ensure!(request.class.is_write(), "read-only call is not an effect");
        let mut slot = self.effects.values().filter(|effect| {
            effect.scope == self.scope()
                && effect.operation == request.operation
                && effect.action == request.action
        });
        let existing = slot.clone().find(|effect| {
            effect.resource == request.resource && effect.payload_hash == request.payload_hash
        });
        if existing.is_none() && request.class != EffectClass::LocalWrite {
            // A variant of an unresolved external write could duplicate its effect.
            ensure!(
                !slot.any(|effect| {
                    matches!(effect.state, EffectState::Started | EffectState::Uncertain)
                }),
                "an effect in this operation is uncertain; reconcile before a new instance"
            );
        }
        if let Some(effect) = existing {
            ensure!(
                effect.class == request.class,
                "effect class changed in current operation"
            );
            ensure!(
                effect.requires_approval == request.requires_approval,
                "effect approval requirement changed in current operation"
            );
            ensure!(
                effect.capability == request.capability,
                "effect capability changed in current operation"
            );
            return match effect.state {
                EffectState::Confirmed => Ok(EffectDecision::Cached {
                    effect_id: effect.effect_id,
                    result: effect
                        .result
                        .as_ref()
                        .expect("confirmed result validated")
                        .payload
                        .clone(),
                }),
                EffectState::NotStarted | EffectState::Failed
                    if effect.class != EffectClass::Destructive =>
                {
                    Ok(EffectDecision::Dispatch {
                        effect_id: effect.effect_id,
                        idempotency_key: effect.idempotency_key.clone(),
                    })
                }
                EffectState::NotStarted => Ok(EffectDecision::Dispatch {
                    effect_id: effect.effect_id,
                    idempotency_key: effect.idempotency_key.clone(),
                }),
                EffectState::Started | EffectState::Uncertain => {
                    bail!("effect is uncertain; reconcile before retry")
                }
                EffectState::Failed => bail!("destructive effect cannot be automatically retried"),
            };
        }
        let effect_id = Uuid::new_v4();
        let idempotency_key = format!("awhdl:{}:{effect_id}", self.run_id());
        self.effects.insert(
            effect_id,
            Effect {
                effect_id,
                scope: self.scope(),
                operation: request.operation.clone(),
                action: request.action.clone(),
                resource: request.resource.clone(),
                payload_hash: request.payload_hash.clone(),
                class: request.class,
                capability: request.capability.clone(),
                idempotency_key: idempotency_key.clone(),
                state: EffectState::NotStarted,
                requires_approval: request.requires_approval,
                invocation_id: None,
                result: None,
            },
        );
        Ok(EffectDecision::Dispatch {
            effect_id,
            idempotency_key,
        })
    }

    pub fn start_effect(&mut self, effect_id: Uuid, identity: &InvocationIdentity) -> Result<()> {
        self.start_effect_at(effect_id, identity, Utc::now())
    }

    /// STARTED and approval consumption are one transition, so they share a snapshot.
    pub fn start_effect_at(
        &mut self,
        effect_id: Uuid,
        identity: &InvocationIdentity,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let effect = self
            .effects
            .get(&effect_id)
            .ok_or_else(|| anyhow::anyhow!("unknown effect"))?;
        ensure!(effect.scope == self.scope(), "stale effect");
        ensure!(
            matches!(effect.state, EffectState::NotStarted | EffectState::Failed),
            "effect is not dispatchable"
        );
        ensure!(
            effect.state != EffectState::Failed || effect.class != EffectClass::Destructive,
            "destructive effect cannot be automatically retried"
        );
        let record = self
            .invocation(identity.invocation_id)
            .ok_or_else(|| anyhow::anyhow!("unknown invocation"))?;
        ensure!(
            &record.identity == identity
                && record.operation == effect.operation
                && record.status == InvocationStatus::Pending
                && identity.generation == effect.scope.generation,
            "effect invocation mismatch"
        );
        if effect.requires_approval {
            self.consume_approval(effect_id, identity.invocation_id, now)?;
        }
        let effect = self.effects.get_mut(&effect_id).expect("checked effect");
        effect.invocation_id = Some(identity.invocation_id);
        effect.state = EffectState::Started;
        effect.result = None;
        Ok(())
    }

    pub fn finish_effect(
        &mut self,
        effect_id: Uuid,
        identity: &InvocationIdentity,
        result: Option<Json>,
    ) -> Result<()> {
        let effect = self
            .effects
            .get(&effect_id)
            .ok_or_else(|| anyhow::anyhow!("unknown effect"))?;
        ensure!(
            effect.scope == self.scope()
                && effect.state == EffectState::Started
                && effect.invocation_id == Some(identity.invocation_id),
            "effect completion mismatch"
        );
        self.finish_result(identity, result.clone())?;
        let effect = self.effects.get_mut(&effect_id).expect("checked effect");
        // A reported local-write error is a definite failure. Remote writes may have
        // committed before the error, so they need reconciliation.
        effect.state = match (&result, effect.class) {
            (Some(_), _) => EffectState::Confirmed,
            (None, EffectClass::LocalWrite) => EffectState::Failed,
            (None, _) => EffectState::Uncertain,
        };
        effect.result = result.map(|payload| InvocationResult { payload });
        Ok(())
    }

    /// Only trusted reconciliation code can call this. It never contacts the provider.
    pub fn reconcile_effect(&mut self, effect_id: Uuid, outcome: Reconciliation) -> Result<()> {
        let effect = self
            .effects
            .get(&effect_id)
            .ok_or_else(|| anyhow::anyhow!("unknown effect"))?;
        ensure!(
            effect.scope == self.scope()
                && matches!(effect.state, EffectState::Started | EffectState::Uncertain),
            "effect is not awaiting reconciliation"
        );
        let invocation_id = effect
            .invocation_id
            .ok_or_else(|| anyhow::anyhow!("effect has no invocation"))?;
        if let Some(record) = self.invocation(invocation_id)
            && record.status == InvocationStatus::Pending
        {
            let identity = record.identity.clone();
            self.finish_result(
                &identity,
                match &outcome {
                    Reconciliation::Confirmed(v) => Some(v.clone()),
                    _ => None,
                },
            )?;
        }
        let effect = self.effects.get_mut(&effect_id).expect("checked effect");
        match outcome {
            Reconciliation::Confirmed(payload) => {
                effect.state = EffectState::Confirmed;
                effect.result = Some(InvocationResult { payload });
            }
            Reconciliation::NotFound => {
                effect.state = EffectState::Failed;
                effect.result = None;
            }
            Reconciliation::StillUncertain => {
                effect.state = EffectState::Uncertain;
            }
        }
        Ok(())
    }

    pub(crate) fn validate_effects(&self) -> Result<()> {
        let mut keys = std::collections::BTreeSet::new();
        let mut slots = std::collections::BTreeSet::new();
        for (id, effect) in &self.effects {
            ensure!(
                *id == effect.effect_id
                    && effect.scope.workflow_run_id == self.run_id()
                    && effect.scope.correlation_id == self.scope().correlation_id
                    && effect.scope.generation <= self.scope().generation
                    && !effect.operation.is_empty()
                    && !effect.action.is_empty()
                    && !effect.resource.is_empty()
                    && effect.payload_hash.len() == 64
                    && effect
                        .payload_hash
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    && effect.class.is_write()
                    && !effect.capability.is_empty()
                    && effect.idempotency_key == format!("awhdl:{}:{id}", self.run_id())
                    && keys.insert(effect.idempotency_key.clone())
                    && slots.insert((
                        effect.scope.generation,
                        &effect.operation,
                        &effect.action,
                        &effect.resource,
                        &effect.payload_hash,
                    )),
                "invalid effect identity"
            );
            ensure!(
                (effect.state == EffectState::Confirmed) == effect.result.is_some(),
                "invalid effect result"
            );
            if let Some(invocation_id) = effect.invocation_id {
                let record = self
                    .invocation(invocation_id)
                    .ok_or_else(|| anyhow::anyhow!("missing effect invocation"))?;
                ensure!(
                    record.operation == effect.operation
                        && record.identity.generation == effect.scope.generation,
                    "invalid effect invocation"
                );
                if effect.state == EffectState::Started {
                    ensure!(
                        record.status == InvocationStatus::Pending,
                        "started effect has terminal invocation"
                    );
                }
            } else {
                ensure!(
                    effect.state == EffectState::NotStarted,
                    "effect has no invocation"
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request(class: EffectClass) -> EffectRequest {
        EffectRequest::new(
            "action:send",
            "send",
            "mail:send",
            &json!({"to":"a"}),
            class,
            None,
            true,
        )
        .unwrap()
        .with_approval(class == EffectClass::Destructive)
        .unwrap()
    }

    #[test]
    fn crash_after_started_requires_reconciliation_before_retry() {
        let mut state = ExecutionState::new(Uuid::new_v4());
        let req = request(EffectClass::ExternalWrite);
        let EffectDecision::Dispatch {
            effect_id,
            idempotency_key,
        } = state.reserve_effect(&req).unwrap()
        else {
            panic!("dispatch")
        };
        assert_eq!(
            state.effect(effect_id).unwrap().state,
            EffectState::NotStarted
        );
        let inv = state.begin(&req.operation, None).unwrap();
        state.start_effect(effect_id, &inv).unwrap();
        let mut restored = ExecutionState::restore(&serde_json::to_vec(&state).unwrap()).unwrap();
        assert_eq!(
            restored.effect(effect_id).unwrap().state,
            EffectState::Uncertain
        );
        assert!(restored.reserve_effect(&req).is_err());
        restored
            .reconcile_effect(effect_id, Reconciliation::StillUncertain)
            .unwrap();
        assert!(restored.reserve_effect(&req).is_err());
        restored
            .reconcile_effect(effect_id, Reconciliation::NotFound)
            .unwrap();
        assert_eq!(
            restored.invocation(inv.invocation_id).unwrap().status,
            InvocationStatus::Failed
        );
        assert_eq!(
            restored.reserve_effect(&req).unwrap(),
            EffectDecision::Dispatch {
                effect_id,
                idempotency_key
            }
        );
        let retry = restored.begin(&req.operation, None).unwrap();
        restored.start_effect(effect_id, &retry).unwrap();
        assert_eq!(retry.attempt, inv.attempt + 1);
        restored
            .finish_effect(effect_id, &retry, Some(json!("sent")))
            .unwrap();
        let mut restored =
            ExecutionState::restore(&serde_json::to_vec(&restored).unwrap()).unwrap();
        assert_eq!(
            restored.reserve_effect(&req).unwrap(),
            EffectDecision::Cached {
                effect_id,
                result: json!("sent")
            }
        );
    }

    fn variant(class: EffectClass, resource: &str, to: &str) -> EffectRequest {
        EffectRequest::new(
            "action:send",
            "send",
            resource,
            &json!({ "to": to }),
            class,
            None,
            true,
        )
        .unwrap()
        .with_approval(class == EffectClass::Destructive)
        .unwrap()
    }

    fn dispatched_id(decision: EffectDecision) -> Uuid {
        let EffectDecision::Dispatch { effect_id, .. } = decision else {
            panic!("dispatch")
        };
        effect_id
    }

    #[test]
    fn unresolved_variant_and_destructive_retry_are_rejected() {
        let mut state = ExecutionState::new(Uuid::new_v4());
        let req = request(EffectClass::Destructive);
        let effect_id = dispatched_id(state.reserve_effect(&req).unwrap());
        let changed = variant(EffectClass::Destructive, "mail:send", "b");
        let destination = variant(EffectClass::Destructive, "other:send", "a");
        let now = Utc::now();
        let approval = state
            .request_approval(
                effect_id,
                &json!({"to":"a"}),
                crate::approval::ApprovalScope::SingleAction,
                now,
                chrono::Duration::minutes(5),
            )
            .unwrap();
        state
            .decide_approval(
                approval.approval_id,
                &approval.action_hash,
                crate::approval::ApprovalDecision::Grant,
                now,
            )
            .unwrap();
        let inv = state.begin(&req.operation, None).unwrap();
        state.start_effect(effect_id, &inv).unwrap();
        state.finish_effect(effect_id, &inv, None).unwrap();
        assert!(state.reserve_effect(&req).is_err());
        assert!(state.reserve_effect(&changed).is_err());
        assert!(state.reserve_effect(&destination).is_err());
        state
            .reconcile_effect(effect_id, Reconciliation::NotFound)
            .unwrap();
        assert!(state.reserve_effect(&req).is_err());
        // Once nothing is unresolved, a different instance gets its own identity.
        let changed_id = dispatched_id(state.reserve_effect(&changed).unwrap());
        assert_ne!(changed_id, effect_id);
        assert_ne!(
            state.effect(changed_id).unwrap().idempotency_key,
            state.effect(effect_id).unwrap().idempotency_key
        );
        assert!(
            state
                .reserve_effect(&variant(EffectClass::ExternalWrite, "mail:send", "b"))
                .is_err()
        );
        ExecutionState::restore(&serde_json::to_vec(&state).unwrap()).unwrap();
        assert!(
            EffectRequest::new(
                "x",
                "a",
                "r",
                &json!({}),
                EffectClass::ExternalWrite,
                None,
                false
            )
            .is_err()
        );
    }

    #[test]
    fn local_write_error_fails_and_allows_retry_or_corrected_instance() {
        let mut state = ExecutionState::new(Uuid::new_v4());
        let req = variant(EffectClass::LocalWrite, "matlab:eval", "x = 1 +");
        let EffectDecision::Dispatch {
            effect_id,
            idempotency_key,
        } = state.reserve_effect(&req).unwrap()
        else {
            panic!("dispatch")
        };
        let inv = state.begin(&req.operation, None).unwrap();
        state.start_effect(effect_id, &inv).unwrap();
        state.finish_effect(effect_id, &inv, None).unwrap();
        assert_eq!(state.effect(effect_id).unwrap().state, EffectState::Failed);
        assert_eq!(
            state.reserve_effect(&req).unwrap(),
            EffectDecision::Dispatch {
                effect_id,
                idempotency_key
            }
        );
        let fixed = variant(EffectClass::LocalWrite, "matlab:eval", "x = 1 + 2");
        let fixed_id = dispatched_id(state.reserve_effect(&fixed).unwrap());
        assert_ne!(fixed_id, effect_id);
        let retry = state.begin(&fixed.operation, None).unwrap();
        state.start_effect(fixed_id, &retry).unwrap();
        state
            .finish_effect(fixed_id, &retry, Some(json!(3)))
            .unwrap();
        let restored = ExecutionState::restore(&serde_json::to_vec(&state).unwrap()).unwrap();
        assert_eq!(
            restored.effect(fixed_id).unwrap().state,
            EffectState::Confirmed
        );
        assert_eq!(
            restored.effect(effect_id).unwrap().state,
            EffectState::Failed
        );
    }

    #[test]
    fn provider_error_remains_uncertain_until_confirmed_by_reconciliation() {
        let mut state = ExecutionState::new(Uuid::new_v4());
        let req = request(EffectClass::ExternalWrite);
        let EffectDecision::Dispatch { effect_id, .. } = state.reserve_effect(&req).unwrap() else {
            panic!("dispatch")
        };
        let inv = state.begin(&req.operation, None).unwrap();
        state.start_effect(effect_id, &inv).unwrap();
        state.finish_effect(effect_id, &inv, None).unwrap();
        assert_eq!(
            state.effect(effect_id).unwrap().state,
            EffectState::Uncertain
        );
        assert_eq!(
            state.invocation(inv.invocation_id).unwrap().status,
            InvocationStatus::Failed
        );
        state
            .reconcile_effect(
                effect_id,
                Reconciliation::Confirmed(json!({"id":"provider-1"})),
            )
            .unwrap();
        assert_eq!(
            state.reserve_effect(&req).unwrap(),
            EffectDecision::Cached {
                effect_id,
                result: json!({"id":"provider-1"}),
            }
        );
        ExecutionState::restore(&serde_json::to_vec(&state).unwrap()).unwrap();
    }
}
