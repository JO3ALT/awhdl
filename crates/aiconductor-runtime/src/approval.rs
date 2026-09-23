//! Human approval bound to one exact Effect instance. The display text shown to a
//! human is never the basis of authorization; only the canonical action hash is.
use crate::effect::{Effect, EffectClass, EffectState, payload_hash, sha256_hex};
use crate::execution::ExecutionState;
use anyhow::{Result, anyhow, ensure};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};
use uuid::Uuid;

/// Version tag of the hashed action descriptor.
pub const ACTION_SCHEMA: &str = "awhdl.action.v1";
pub const MAX_APPROVAL_TTL: Duration = Duration::hours(24);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalScope {
    SingleAction,
    /// Defined for policy vocabulary; not accepted by this runtime.
    Transaction,
    /// Defined for policy vocabulary; not accepted by this runtime.
    TimeLimitedSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalState {
    Pending,
    Granted,
    Denied,
    Consumed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    Grant,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Approval {
    pub approval_id: Uuid,
    pub effect_id: Uuid,
    pub scope: ApprovalScope,
    pub action_hash: String,
    pub content_hash: String,
    pub capability: String,
    pub resource: String,
    pub class: EffectClass,
    pub generation: u64,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub state: ApprovalState,
    pub decided_at: Option<DateTime<Utc>>,
    /// Invocation whose dispatch consumed this approval.
    pub consumed_by: Option<Uuid>,
}

/// What a Human device receives. `canonical_action` is the approval target;
/// `display` is a convenience rendering and must not be used as evidence.
#[derive(Debug, Clone, Serialize)]
pub struct ApprovalRequest {
    pub approval_id: Uuid,
    pub action_hash: String,
    pub expires_at: DateTime<Utc>,
    pub canonical_action: Json,
    pub display: String,
}

impl ExecutionState {
    pub fn approval(&self, id: Uuid) -> Option<&Approval> {
        self.approvals.get(&id)
    }

    pub fn has_granted_approval(&self, effect_id: Uuid, now: DateTime<Utc>) -> bool {
        self.approvals.values().any(|a| {
            a.effect_id == effect_id && a.state == ApprovalState::Granted && now <= a.expires_at
        })
    }

    pub fn approval_consumed_by(&self, invocation_id: Uuid) -> Option<Uuid> {
        self.approvals
            .values()
            .find(|a| a.consumed_by == Some(invocation_id))
            .map(|a| a.approval_id)
    }

    /// Everything that invalidates an approval when changed: run, generation,
    /// action instance, destination, permission scope, destructive level, content.
    fn action_descriptor(&self, effect: &Effect) -> Result<Json> {
        Ok(json!({
            "schema": ACTION_SCHEMA,
            "workflow_run_id": effect.scope.workflow_run_id,
            "correlation_id": effect.scope.correlation_id,
            "generation": effect.scope.generation,
            "effect_id": effect.effect_id,
            "operation": effect.operation,
            "action": effect.action,
            "capability": effect.capability,
            "resource": effect.resource,
            "class": effect.class,
            "content_hash": effect.payload_hash,
        }))
    }

    fn action_hash(&self, effect: &Effect) -> Result<String> {
        Ok(sha256_hex(&serde_json::to_vec(
            &self.action_descriptor(effect)?,
        )?))
    }

    fn live_approval(&self, approval: &Approval, now: DateTime<Utc>) -> bool {
        matches!(
            approval.state,
            ApprovalState::Pending | ApprovalState::Granted
        ) && now <= approval.expires_at
    }

    /// `parameters` must be the exact arguments that will be dispatched; they are
    /// shown to the human and checked against the Effect's content hash.
    pub fn request_approval(
        &mut self,
        effect_id: Uuid,
        parameters: &Json,
        scope: ApprovalScope,
        now: DateTime<Utc>,
        ttl: Duration,
    ) -> Result<ApprovalRequest> {
        ensure!(
            scope == ApprovalScope::SingleAction,
            "approval scope is not permitted by policy"
        );
        ensure!(
            ttl > Duration::zero() && ttl <= MAX_APPROVAL_TTL,
            "invalid approval lifetime"
        );
        let effect = self
            .effects
            .get(&effect_id)
            .ok_or_else(|| anyhow!("unknown effect"))?;
        ensure!(effect.scope == self.scope(), "stale effect");
        ensure!(effect.requires_approval, "effect does not require approval");
        ensure!(
            effect.state == EffectState::NotStarted
                || (effect.state == EffectState::Failed
                    && effect.class != EffectClass::Destructive),
            "effect is not dispatchable"
        );
        ensure!(
            payload_hash(parameters)? == effect.payload_hash,
            "approval parameters do not match effect content"
        );
        ensure!(
            !self
                .approvals
                .values()
                .any(|a| a.effect_id == effect_id && self.live_approval(a, now)),
            "effect already has a live approval"
        );
        let descriptor = self.action_descriptor(effect)?;
        let action_hash = self.action_hash(effect)?;
        let approval = Approval {
            approval_id: Uuid::new_v4(),
            effect_id,
            scope,
            action_hash: action_hash.clone(),
            content_hash: effect.payload_hash.clone(),
            capability: effect.capability.clone(),
            resource: effect.resource.clone(),
            class: effect.class,
            generation: effect.scope.generation,
            issued_at: now,
            expires_at: now + ttl,
            state: ApprovalState::Pending,
            decided_at: None,
            consumed_by: None,
        };
        let display = format!(
            "{} ({}) on {}",
            effect.action, approval.capability, effect.resource
        );
        let mut canonical_action = descriptor;
        canonical_action["parameters"] = parameters.clone();
        let request = ApprovalRequest {
            approval_id: approval.approval_id,
            action_hash,
            expires_at: approval.expires_at,
            canonical_action,
            display,
        };
        self.approvals.insert(approval.approval_id, approval);
        Ok(request)
    }

    /// Trusted approval adapters call this. The decision must echo the hash of the
    /// canonical action the human saw, and that action must still be current.
    pub fn decide_approval(
        &mut self,
        approval_id: Uuid,
        action_hash: &str,
        decision: ApprovalDecision,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let approval = self
            .approvals
            .get(&approval_id)
            .ok_or_else(|| anyhow!("unknown approval"))?;
        ensure!(
            approval.state == ApprovalState::Pending,
            "approval already decided"
        );
        ensure!(now <= approval.expires_at, "approval expired");
        ensure!(
            approval.action_hash == action_hash,
            "decision is for a different action"
        );
        let effect = self
            .effects
            .get(&approval.effect_id)
            .ok_or_else(|| anyhow!("unknown effect"))?;
        ensure!(
            effect.scope == self.scope() && self.action_hash(effect)? == approval.action_hash,
            "approved action is stale"
        );
        let approval = self.approvals.get_mut(&approval_id).expect("checked");
        approval.state = match decision {
            ApprovalDecision::Grant => ApprovalState::Granted,
            ApprovalDecision::Deny => ApprovalState::Denied,
        };
        approval.decided_at = Some(now);
        Ok(())
    }

    /// Single use: consumed in the same transition that marks the Effect STARTED.
    pub(crate) fn consume_approval(
        &mut self,
        effect_id: Uuid,
        invocation_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<Uuid> {
        let effect = self
            .effects
            .get(&effect_id)
            .ok_or_else(|| anyhow!("unknown effect"))?;
        let expected = self.action_hash(effect)?;
        let approval_id = self
            .approvals
            .values()
            .find(|a| {
                a.effect_id == effect_id
                    && a.state == ApprovalState::Granted
                    && now <= a.expires_at
                    && a.action_hash == expected
            })
            .map(|a| a.approval_id)
            .ok_or_else(|| {
                anyhow!("effect requires a granted, unexpired approval bound to this action")
            })?;
        let approval = self.approvals.get_mut(&approval_id).expect("found");
        approval.state = ApprovalState::Consumed;
        approval.consumed_by = Some(invocation_id);
        Ok(approval_id)
    }

    pub(crate) fn validate_approvals(&self) -> Result<()> {
        let mut consumers = std::collections::BTreeSet::new();
        for (id, approval) in &self.approvals {
            let effect = self
                .effects
                .get(&approval.effect_id)
                .ok_or_else(|| anyhow!("approval for unknown effect"))?;
            ensure!(
                *id == approval.approval_id
                    && approval.scope == ApprovalScope::SingleAction
                    && effect.requires_approval
                    && approval.generation == effect.scope.generation
                    && approval.content_hash == effect.payload_hash
                    && approval.resource == effect.resource
                    && approval.class == effect.class
                    && approval.capability == effect.capability
                    && approval.action_hash == self.action_hash(effect)?
                    && approval.expires_at > approval.issued_at
                    && approval.expires_at - approval.issued_at <= MAX_APPROVAL_TTL,
                "invalid approval binding"
            );
            ensure!(
                (approval.state == ApprovalState::Pending) == approval.decided_at.is_none()
                    && (approval.state == ApprovalState::Consumed)
                        == approval.consumed_by.is_some(),
                "invalid approval state"
            );
            if let Some(invocation_id) = approval.consumed_by {
                let record = self
                    .invocation(invocation_id)
                    .ok_or_else(|| anyhow!("approval consumed by unknown invocation"))?;
                ensure!(
                    record.operation == effect.operation && consumers.insert(invocation_id),
                    "invalid approval consumption"
                );
            }
        }
        // Each dispatch of an approval-bound Effect consumed exactly one approval.
        for effect in self.effects.values() {
            if effect.requires_approval
                && let Some(invocation_id) = effect.invocation_id
            {
                ensure!(
                    consumers.contains(&invocation_id),
                    "approval-bound effect dispatched without approval"
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect::{EffectDecision, EffectRequest, Reconciliation};

    fn params(to: &str) -> Json {
        json!({"to": to, "branch": "main", "commit": "123abc"})
    }

    fn request(class: EffectClass, to: &str) -> EffectRequest {
        EffectRequest::new(
            "action:push",
            "push",
            "github:push",
            &params(to),
            class,
            None,
            true,
        )
        .unwrap()
    }

    fn reserve(state: &mut ExecutionState, req: &EffectRequest) -> Uuid {
        let EffectDecision::Dispatch { effect_id, .. } = state.reserve_effect(req).unwrap() else {
            panic!("dispatch")
        };
        effect_id
    }

    fn grant(state: &mut ExecutionState, effect_id: Uuid, to: &str, now: DateTime<Utc>) -> Uuid {
        let request = state
            .request_approval(
                effect_id,
                &params(to),
                ApprovalScope::SingleAction,
                now,
                Duration::minutes(10),
            )
            .unwrap();
        assert_eq!(request.canonical_action["parameters"], params(to));
        state
            .decide_approval(
                request.approval_id,
                &request.action_hash,
                ApprovalDecision::Grant,
                now,
            )
            .unwrap();
        request.approval_id
    }

    /// Mirrors `RunStore`: begin and start commit together or not at all.
    fn dispatch(
        state: &mut ExecutionState,
        req: &EffectRequest,
        effect_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<crate::execution::InvocationIdentity> {
        let mut next = state.clone();
        let inv = next.begin(req.operation(), None)?;
        next.start_effect_at(effect_id, &inv, now)?;
        *state = next;
        Ok(inv)
    }

    #[test]
    fn external_write_needs_bound_approval_and_consumes_it_once() {
        let now = Utc::now();
        let mut state = ExecutionState::new(Uuid::new_v4());
        let req = request(EffectClass::ExternalWrite, "origin");
        let effect_id = reserve(&mut state, &req);
        // No approval: fail closed and leave nothing started.
        assert!(dispatch(&mut state, &req, effect_id, now).is_err());
        assert_eq!(
            state.effect(effect_id).unwrap().state,
            EffectState::NotStarted
        );
        let approval_id = grant(&mut state, effect_id, "origin", now);
        let inv = dispatch(&mut state, &req, effect_id, now).unwrap();
        let approval = state.approval(approval_id).unwrap();
        assert_eq!(approval.state, ApprovalState::Consumed);
        assert_eq!(approval.consumed_by, Some(inv.invocation_id));
        state.finish_effect(effect_id, &inv, None).unwrap();
        state
            .reconcile_effect(effect_id, Reconciliation::NotFound)
            .unwrap();
        // Replay: the consumed approval cannot authorize the retry.
        assert!(dispatch(&mut state, &req, effect_id, now).is_err());
        grant(&mut state, effect_id, "origin", now);
        dispatch(&mut state, &req, effect_id, now).unwrap();
        ExecutionState::restore(&serde_json::to_vec(&state).unwrap()).unwrap();
    }

    #[test]
    fn changed_payload_or_commit_does_not_inherit_approval() {
        let now = Utc::now();
        let mut state = ExecutionState::new(Uuid::new_v4());
        let req = request(EffectClass::ExternalWrite, "origin");
        let effect_id = reserve(&mut state, &req);
        // Approval request must carry the exact dispatched content.
        assert!(
            state
                .request_approval(
                    effect_id,
                    &params("fork"),
                    ApprovalScope::SingleAction,
                    now,
                    Duration::minutes(10),
                )
                .is_err()
        );
        grant(&mut state, effect_id, "origin", now);
        let mut changed_commit = params("origin");
        changed_commit["commit"] = json!("456def");
        let changed = EffectRequest::new(
            "action:push",
            "push",
            "github:push",
            &changed_commit,
            EffectClass::ExternalWrite,
            None,
            true,
        )
        .unwrap();
        let changed_id = reserve(&mut state, &changed);
        assert_ne!(changed_id, effect_id);
        assert!(dispatch(&mut state, &changed, changed_id, now).is_err());
        let destination = request(EffectClass::ExternalWrite, "fork");
        let destination_id = reserve(&mut state, &destination);
        assert!(dispatch(&mut state, &destination, destination_id, now).is_err());
    }

    #[test]
    fn expiry_generation_change_and_wrong_hash_invalidate_approval() {
        let now = Utc::now();
        let mut state = ExecutionState::new(Uuid::new_v4());
        let req = request(EffectClass::Destructive, "origin");
        let effect_id = reserve(&mut state, &req);
        let pending = state
            .request_approval(
                effect_id,
                &params("origin"),
                ApprovalScope::SingleAction,
                now,
                Duration::minutes(10),
            )
            .unwrap();
        assert!(
            state
                .decide_approval(
                    pending.approval_id,
                    &"0".repeat(64),
                    ApprovalDecision::Grant,
                    now
                )
                .is_err()
        );
        let later = now + Duration::minutes(11);
        assert!(
            state
                .decide_approval(
                    pending.approval_id,
                    &pending.action_hash,
                    ApprovalDecision::Grant,
                    later
                )
                .is_err()
        );
        // The expired pending request no longer blocks a fresh one.
        grant(&mut state, effect_id, "origin", later);
        assert!(dispatch(&mut state, &req, effect_id, later + Duration::minutes(11)).is_err());

        // A new generation invalidates the approval and yields a new, unapproved instance.
        let mut state = ExecutionState::new(Uuid::new_v4());
        let effect_id = reserve(&mut state, &req);
        let approval_id = grant(&mut state, effect_id, "origin", now);
        state.advance_generation().unwrap();
        assert!(dispatch(&mut state, &req, effect_id, now).is_err());
        let next = reserve(&mut state, &req);
        assert_ne!(next, effect_id);
        assert!(dispatch(&mut state, &req, next, now).is_err());
        assert_eq!(
            state.approval(approval_id).unwrap().state,
            ApprovalState::Granted
        );
    }

    #[test]
    fn broad_scopes_denials_and_opt_outs_are_policy_checked() {
        let now = Utc::now();
        let mut state = ExecutionState::new(Uuid::new_v4());
        let req = request(EffectClass::ExternalWrite, "origin");
        let effect_id = reserve(&mut state, &req);
        for scope in [
            ApprovalScope::Transaction,
            ApprovalScope::TimeLimitedSession,
        ] {
            assert!(
                state
                    .request_approval(
                        effect_id,
                        &params("origin"),
                        scope,
                        now,
                        Duration::minutes(10)
                    )
                    .is_err()
            );
        }
        let denied = state
            .request_approval(
                effect_id,
                &params("origin"),
                ApprovalScope::SingleAction,
                now,
                Duration::minutes(10),
            )
            .unwrap();
        state
            .decide_approval(
                denied.approval_id,
                &denied.action_hash,
                ApprovalDecision::Deny,
                now,
            )
            .unwrap();
        assert!(dispatch(&mut state, &req, effect_id, now).is_err());
        assert!(
            request(EffectClass::Destructive, "origin")
                .with_approval(false)
                .is_err()
        );
        let opted_out = request(EffectClass::ExternalWrite, "mirror")
            .with_approval(false)
            .unwrap();
        // Same instance cannot silently change its approval requirement.
        let mirror = request(EffectClass::ExternalWrite, "mirror");
        reserve(&mut state, &mirror);
        assert!(state.reserve_effect(&opted_out).is_err());
        let local = request(EffectClass::LocalWrite, "local");
        assert!(!local.requires_approval());
        let local_id = reserve(&mut state, &local);
        dispatch(&mut state, &local, local_id, now).unwrap();
        ExecutionState::restore(&serde_json::to_vec(&state).unwrap()).unwrap();
    }

    #[test]
    fn restore_rejects_tampered_approval_binding() {
        let now = Utc::now();
        let mut state = ExecutionState::new(Uuid::new_v4());
        let req = request(EffectClass::ExternalWrite, "origin");
        let effect_id = reserve(&mut state, &req);
        let approval_id = grant(&mut state, effect_id, "origin", now);
        let mut encoded = serde_json::to_value(&state).unwrap();
        encoded["approvals"][approval_id.to_string()]["action_hash"] = json!("f".repeat(64));
        assert!(ExecutionState::restore(&serde_json::to_vec(&encoded).unwrap()).is_err());
        let mut encoded = serde_json::to_value(&state).unwrap();
        encoded["effects"][effect_id.to_string()]["requires_approval"] = json!(false);
        assert!(ExecutionState::restore(&serde_json::to_vec(&encoded).unwrap()).is_err());
        // A different capability (permission scope) breaks the approval binding.
        let mut encoded = serde_json::to_value(&state).unwrap();
        encoded["effects"][effect_id.to_string()]["capability"] = json!("other#0123456789abcdef");
        assert!(ExecutionState::restore(&serde_json::to_vec(&encoded).unwrap()).is_err());
        let rescoped = request(EffectClass::ExternalWrite, "origin")
            .with_capability("other#0123456789abcdef")
            .unwrap();
        assert!(state.reserve_effect(&rescoped).is_err());
    }
}
