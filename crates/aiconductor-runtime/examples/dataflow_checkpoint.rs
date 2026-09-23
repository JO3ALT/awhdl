//! Offline example: emit a v5 checkpoint without contacting any device.
use aiconductor::approval::{ApprovalDecision, ApprovalScope};
use aiconductor::capability::{Capability, CapabilityPolicy, CapabilityRequest};
use aiconductor::effect::{EffectClass, EffectDecision, EffectRequest};
use aiconductor::execution::ExecutionState;
use anyhow::Result;
use chrono::{Duration, Utc};
use serde_json::json;
use uuid::Uuid;

fn main() -> Result<()> {
    let policy = CapabilityPolicy {
        schema_version: 1,
        grants: vec![Capability {
            id: "mail_send".to_owned(),
            subjects: vec!["send".to_owned()],
            action: "mcp.call".to_owned(),
            resource: "mcp:mail/send".to_owned(),
            constraints: Default::default(),
            effect_class: EffectClass::ExternalWrite,
            revoked: false,
        }],
    };
    policy.validate()?;
    let authorization =
        policy.authorize("send", &CapabilityRequest::new("mcp.call", "mcp:mail/send"))?;
    let mut state = ExecutionState::new(Uuid::new_v4());
    let request = state.publish("request", json!("build"), None)?;
    state.consume(request)?;
    state.assign("candidate", json!("first"))?;
    state.assign("candidate", json!("second"))?;
    let build = state.begin_caused("build", None, Some(request))?;
    state.finish_result(&build, Some(json!({"ok": true})))?;
    let failed = state.begin("review", None)?;
    state.finish(&failed, false)?;
    let retry = state.begin("review", None)?;
    state.cancel(&retry)?;
    state.begin("pending", None)?;
    let effect_request = EffectRequest::new(
        "action:send",
        "send",
        "mail:send",
        &json!({"to":"alice"}),
        authorization.effect_class,
        None,
        true,
    )?
    .with_capability(&authorization.handle)?;
    let EffectDecision::Dispatch { effect_id, .. } = state.reserve_effect(&effect_request)? else {
        anyhow::bail!("unexpected cached effect");
    };
    let now = Utc::now();
    let approval = state.request_approval(
        effect_id,
        &json!({"to":"alice"}),
        ApprovalScope::SingleAction,
        now,
        Duration::minutes(10),
    )?;
    state.decide_approval(
        approval.approval_id,
        &approval.action_hash,
        ApprovalDecision::Grant,
        now,
    )?;
    let send = state.begin("action:send", None)?;
    state.start_effect_at(effect_id, &send, now)?;
    state.finish_effect(effect_id, &send, Some(json!({"provider_id":"123"})))?;
    // A second instance awaiting a human decision.
    let pending = EffectRequest::new(
        "action:send",
        "send",
        "mail:send",
        &json!({"to":"bob"}),
        authorization.effect_class,
        None,
        true,
    )?
    .with_capability(&authorization.handle)?;
    let EffectDecision::Dispatch { effect_id, .. } = state.reserve_effect(&pending)? else {
        anyhow::bail!("unexpected cached effect");
    };
    state.request_approval(
        effect_id,
        &json!({"to":"bob"}),
        ApprovalScope::SingleAction,
        now,
        Duration::minutes(10),
    )?;
    let encoded = serde_json::to_vec_pretty(&state)?;
    let mut restored = ExecutionState::restore(&encoded)?;
    assert!(restored.consume(request)?.is_none());
    println!("{}", String::from_utf8(encoded)?);
    Ok(())
}
