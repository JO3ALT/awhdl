//! Live MATLAB check through the runtime's MCP path without the planner/LLM:
//! adapter preparation, capability authorization, Effect journal, MCP call and
//! normalization, exactly as the engine dispatches `calculation_graphing`.
//! Requires the MATLAB MCP host. Usage: live_matlab_smoke <project root>
use aiconductor::adapters::{AdapterInput, normalize_result, prepare_call};
use aiconductor::audit::RunStore;
use aiconductor::budget::BudgetUsage;
use aiconductor::capability::CapabilityRequest;
use aiconductor::config::ProjectConfig;
use aiconductor::effect::EffectRequest;
use aiconductor::mcp::McpClient;
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::time::Instant;

const ACTION: &str = "calculation_graphing";

async fn call(
    config: &ProjectConfig,
    store: &mut RunStore,
    tool: &str,
    arguments: Value,
) -> Result<String> {
    let route = config.action(ACTION).context("route")?;
    let server_name = route.server.as_deref().context("server")?;
    let server = config
        .mcp
        .servers
        .get(server_name)
        .context("server config")?;
    let prepared = prepare_call(
        config,
        AdapterInput {
            action: ACTION,
            route,
            requested_tool: Some(tool),
            input: "",
            source_instruction: "",
            arguments,
        },
    )?;
    let authorization = config.authorize_route(
        ACTION,
        &CapabilityRequest::new("mcp.call", format!("mcp:{server_name}/{}", prepared.tool)),
    )?;
    let class = authorization.effect_class;
    let request = EffectRequest::new(
        &format!("action:{ACTION}"),
        ACTION,
        &format!("{server_name}:{}", prepared.tool),
        &prepared.arguments,
        class,
        route.idempotency_argument.clone(),
        route.manual_reconciliation,
    )?
    .with_approval(route.requires_human_approval_for(class))?
    .with_capability(&authorization.handle)?;
    let limit = config.runtime.planner.max_observation_bytes;
    let (adapter, tool, arguments) = (prepared.adapter, prepared.tool, prepared.arguments);
    store
        .invoke_effect(request, &BudgetUsage::default(), |_key| async move {
            let raw = McpClient::call_tool(config, server, &tool, arguments).await?;
            normalize_result(adapter, &tool, &raw, limit)?.for_planner()
        })
        .await
}

fn report(step: &str, started: Instant, outcome: &Result<String>) {
    let text = match outcome {
        Ok(text) => format!("OK {}", text.chars().take(400).collect::<String>()),
        Err(error) => format!("ERR {error:#}"),
    };
    println!("[{step}] {:.1}s {text}", started.elapsed().as_secs_f64());
}

#[tokio::main]
async fn main() -> Result<()> {
    let root = std::env::args().nth(1).unwrap_or_else(|| ".".to_owned());
    let config = ProjectConfig::load(&root)?;
    let mut store = RunStore::create(&config.root, &config.runtime.run_root, "live MATLAB smoke")?;
    println!("run dir: {}", store.dir().display());
    let steps: [(&str, &str, Value); 5] = [
        (
            "1 compute",
            "evaluate_matlab_code",
            json!({"code": "x = sum(1:10); fprintf('AIC_MATLAB_OK %d\\n', x);"}),
        ),
        (
            "2 duplicate (expect cached)",
            "evaluate_matlab_code",
            json!({"code": "x = sum(1:10); fprintf('AIC_MATLAB_OK %d\\n', x);"}),
        ),
        (
            "3 error (expect FAILED)",
            "evaluate_matlab_code",
            json!({"code": "y = aic_undefined_function_xyz(1);"}),
        ),
        (
            "4 corrected",
            "evaluate_matlab_code",
            json!({"code": "y = sqrt(16); fprintf('AIC_MATLAB_FIXED %g\\n', y);"}),
        ),
        (
            "5 run file",
            "run_matlab_file",
            json!({"script_path": "examples/cvim_full_loop_test/full_loop_probe.m"}),
        ),
    ];
    for (step, tool, arguments) in steps {
        let started = Instant::now();
        let outcome = call(&config, &mut store, tool, arguments).await;
        report(step, started, &outcome);
    }
    let started = Instant::now();
    let denied = call(
        &config,
        &mut store,
        "run_matlab_file",
        json!({"script_path": "config/runtime.toml"}),
    )
    .await;
    report(
        "6 ungranted file (expect denied, no MATLAB call)",
        started,
        &denied,
    );
    for effect in store.execution().effects() {
        println!(
            "effect {} {:?} class={:?} capability={} approval={}",
            &effect.payload_hash[..12],
            effect.state,
            effect.class,
            effect.capability,
            effect.requires_approval
        );
    }
    Ok(())
}
