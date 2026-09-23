//! Live check of the HumanPort approval GUI. The approved "write" is a dummy
//! closure: nothing external happens. Usage: live_approval_smoke <project root>
use aiconductor::approval::ApprovalDecision;
use aiconductor::approver::{Approver, HumanPortApprover};
use aiconductor::audit::RunStore;
use aiconductor::budget::BudgetUsage;
use aiconductor::config::ProjectConfig;
use aiconductor::effect::{EffectClass, EffectDecision, EffectRequest};
use anyhow::{Context, Result};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<()> {
    let root = std::env::args().nth(1).unwrap_or_else(|| ".".to_owned());
    let config = ProjectConfig::load(&root)?;
    let settings = config
        .runtime
        .approval
        .clone()
        .context("no [approval] adapter")?;
    let approver = HumanPortApprover::new(&config, &settings)?;
    let mut store = RunStore::create(
        &config.root,
        &config.runtime.run_root,
        "live approval smoke",
    )?;
    let usage = BudgetUsage::default();
    let parameters =
        json!({"demo": "approval smoke test", "note": "no external action is performed"});
    let request = || {
        EffectRequest::new(
            "action:approval_smoke",
            "approval_smoke",
            "demo:approval-smoke",
            &parameters,
            EffectClass::ExternalWrite,
            None,
            true,
        )
    };
    let EffectDecision::Dispatch { effect_id, .. } = store.reserve_effect(&request()?)? else {
        anyhow::bail!("unexpected cached effect");
    };
    let pending = store.request_approval(
        effect_id,
        &parameters,
        chrono::Duration::seconds(settings.ttl_sec),
        &usage,
    )?;
    println!("run dir: {}", store.dir().display());
    println!(
        "waiting for the HumanPort window (action_hash {})",
        pending.action_hash
    );
    let answer = approver.ask(&pending).await?;
    println!("human answered: {:?}", answer.decision);
    store.decide_approval(
        pending.approval_id,
        &answer.echoed_action_hash,
        answer.decision,
        &usage,
    )?;
    let dispatched = store
        .invoke_effect(request()?, &usage, |_| async {
            Ok("dummy write executed".to_owned())
        })
        .await;
    let effect = store.execution().effect(effect_id).context("effect")?;
    println!(
        "dispatch: {:?}; effect state: {:?}; approval state: {:?}",
        dispatched.map_err(|e| e.to_string()),
        effect.state,
        store
            .execution()
            .approval(pending.approval_id)
            .map(|a| a.state)
    );
    if answer.decision == ApprovalDecision::Deny {
        println!("denied: the provider closure was not called");
    }
    Ok(())
}
