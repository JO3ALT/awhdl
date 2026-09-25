//! AWHDL designs compiled to a runtime program and executed with delta cycles.
//! Devices are bound to configured routes, so every call keeps the engine's
//! capability, Effect, approval and audit path.
use crate::config::ProjectConfig;
use anyhow::{Context, Result, bail, ensure};
use async_trait::async_trait;
use awhdl_ast::{
    Assertion, BinaryOp, BudgetValue, ConcurrentStatement, Declaration, DesignUnit, DeviceCall,
    Expr, Expression, PortMode, ProcessStmt, SequentialStatement,
};
use awhdl_checker::{Class, Location, class_of, location_of, parse_class};
use serde::Serialize;
use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

/// A device bound to a configured route.
#[derive(Debug, Clone, Serialize)]
pub struct BoundDevice {
    pub name: String,
    pub kind: String,
    pub route: String,
    pub location: Location,
    pub clearance: Option<Class>,
}

/// A declassifier: the range it may release and the trusted means that must
/// all allow a release (SECURITY_SPEC, declassification).
#[derive(Debug, Clone, Serialize)]
pub struct Declassifier {
    pub name: String,
    pub from: Class,
    pub to: Class,
    /// A local deterministic device and the method that must return `{"pass": true}`.
    pub filter: Option<(String, String)>,
    /// Human approval of the exact content.
    pub approval: bool,
}

/// What a human is asked to release: the exact content, never only a summary.
#[derive(Debug, Clone, Serialize)]
pub struct ReleaseRequest {
    pub declassifier: String,
    pub from: Class,
    pub to: Class,
    pub source: String,
    pub target: String,
    /// Where the release happens: the process that runs it and the statement,
    /// so the human judges the release in its context, not only its content.
    pub context: String,
    pub content: Json,
    pub content_sha256: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SignalInfo {
    pub class: Class,
    pub initial: Json,
    /// `Some(mode)` for entity ports.
    pub port: Option<PortMode>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct DesignLimits {
    pub iterations: Option<u32>,
    pub wall_time_ms: Option<u64>,
    pub tool_calls: Option<u32>,
    pub model_calls: Option<u32>,
}

/// The runtime program for one entity/architecture pair.
#[derive(Debug, Clone, Serialize)]
pub struct CompiledDesign {
    pub entity: String,
    pub architecture: String,
    pub signals: BTreeMap<String, SignalInfo>,
    pub devices: BTreeMap<String, BoundDevice>,
    pub declassifiers: BTreeMap<String, Declassifier>,
    pub processes: Vec<ProcessStmt>,
    pub assertions: Vec<Assertion>,
    /// Lowest class that may never reach a cloud device.
    pub cloud_floor: Class,
    pub timers: BTreeMap<String, u64>,
    pub barriers: BTreeMap<String, Vec<String>>,
    pub limits: DesignLimits,
}

const DEVICE_KINDS: [&str; 3] = ["agent", "mcp", "deterministic"];

/// Parse, statically check and bind a design. Any static diagnostic is fatal.
pub fn compile(source: &str, config: &ProjectConfig) -> Result<CompiledDesign> {
    let design = awhdl_parser::parse(source).map_err(|error| anyhow::anyhow!("{error}"))?;
    let diagnostics = awhdl_checker::check(&design);
    if !diagnostics.is_empty() {
        let rendered = diagnostics
            .iter()
            .map(|item| format!("{}: {}", item.code, item.message))
            .collect::<Vec<_>>()
            .join("; ");
        bail!("static check failed: {rendered}");
    }
    let architectures = design
        .units
        .iter()
        .filter_map(|unit| match unit {
            DesignUnit::Architecture(architecture) => Some(architecture),
            _ => None,
        })
        .collect::<Vec<_>>();
    ensure!(
        architectures.len() == 1,
        "a runnable design needs exactly one architecture (configurations are not in the v0.1 profile)"
    );
    let architecture = architectures[0];
    let entity = design
        .units
        .iter()
        .find_map(|unit| match unit {
            DesignUnit::Entity(entity) if entity.name == architecture.entity => Some(entity),
            _ => None,
        })
        .context("checked architecture has an entity")?;

    let mut signals = BTreeMap::new();
    for port in &entity.ports {
        for name in &port.names {
            signals.insert(
                name.clone(),
                SignalInfo {
                    class: class_of(&port.data_type),
                    initial: Json::Null,
                    port: Some(port.mode),
                },
            );
        }
    }
    let mut devices = BTreeMap::new();
    let mut declassifiers = BTreeMap::new();
    let mut timers = BTreeMap::new();
    let mut barriers = BTreeMap::new();
    let mut limits = DesignLimits::default();
    for declaration in &architecture.declarations {
        match declaration {
            Declaration::Signal(signal) => {
                let initial = match &signal.initial {
                    Some(expression) => constant(&expression.expr)
                        .with_context(|| format!("initial value {}", expression.source))?,
                    None => Json::Null,
                };
                for name in &signal.names {
                    signals.insert(
                        name.clone(),
                        SignalInfo {
                            class: class_of(&signal.data_type),
                            initial: initial.clone(),
                            port: None,
                        },
                    );
                }
            }
            // A declassifier has no route: it is a policy over other devices.
            Declaration::Device(device) if device.device_type == "declassifier" => {
                declassifiers.insert(device.name.clone(), declassifier(device)?);
            }
            Declaration::Device(device) => {
                devices.insert(device.name.clone(), bind(device, config)?);
            }
            Declaration::Timer(timer) => {
                timers.insert(timer.name.clone(), timer.period.millis);
            }
            Declaration::Barrier(barrier) => {
                barriers.insert(barrier.name.clone(), barrier.members.clone());
            }
            Declaration::Budget(budget) => {
                for limit in &budget.limits {
                    match (limit.key.as_str(), limit.value) {
                        ("iterations", BudgetValue::Count(n)) => {
                            limits.iterations = Some(u32::try_from(n)?)
                        }
                        ("tool_calls", BudgetValue::Count(n)) => {
                            limits.tool_calls = Some(u32::try_from(n)?)
                        }
                        ("model_calls", BudgetValue::Count(n)) => {
                            limits.model_calls = Some(u32::try_from(n)?)
                        }
                        ("wall_time", BudgetValue::Time(time)) => {
                            limits.wall_time_ms = Some(time.millis)
                        }
                        (key, _) => bail!(
                            "budget limit {key} is not enforced by the v0.1 runtime; remove it"
                        ),
                    }
                }
            }
        }
    }
    ensure!(
        timers.is_empty() || limits.iterations.is_some() || limits.wall_time_ms.is_some(),
        "a timer-driven design needs an iterations or wall_time budget"
    );
    let mut processes = Vec::new();
    let mut assertions = Vec::new();
    let mut cloud_floor = Class::Restricted;
    for statement in &architecture.statements {
        match statement {
            ConcurrentStatement::Process(process) => processes.push(process.clone()),
            ConcurrentStatement::Assert(assert) => {
                if let Assertion::NeverFlow {
                    classification,
                    location,
                } = &assert.assertion
                {
                    if location == "cloud"
                        && let Some(class) = parse_class(classification)
                    {
                        cloud_floor = cloud_floor.min(class);
                    }
                } else {
                    assertions.push(assert.assertion.clone());
                }
            }
        }
    }
    Ok(CompiledDesign {
        entity: entity.name.clone(),
        architecture: architecture.name.clone(),
        signals,
        devices,
        declassifiers,
        processes,
        assertions,
        cloud_floor,
        timers,
        barriers,
        limits,
    })
}

/// Bind a device to its route: kind and location must agree with configuration,
/// otherwise the static flow check would reason about the wrong location.
fn bind(device: &awhdl_ast::DeviceDecl, config: &ProjectConfig) -> Result<BoundDevice> {
    ensure!(
        DEVICE_KINDS.contains(&device.device_type.as_str()),
        "device {} has kind {}; v0.1 devices are agent, mcp or deterministic",
        device.name,
        device.device_type
    );
    let route_name = match device.generic("route") {
        Some(awhdl_ast::GenericValue::String(route)) => route.clone(),
        _ => bail!(
            "device {} needs generic (route => \"<configured action>\")",
            device.name
        ),
    };
    let route = config.action(&route_name).with_context(|| {
        format!(
            "device {} is bound to unknown route {route_name}",
            device.name
        )
    })?;
    ensure!(
        route_name != config.routes.default_action,
        "device {} cannot bind the planner route",
        device.name
    );
    let codex = route.adapter == Some(crate::config::McpAdapterKind::Codex);
    let compatible = match device.device_type.as_str() {
        "agent" => route.kind == "model" || codex,
        "mcp" => route.kind == "mcp" && !codex,
        _ => route.kind == "mcp" && !codex && !route.effective_class().is_write(),
    };
    ensure!(
        compatible,
        "device {} of kind {} cannot bind route {route_name}",
        device.name,
        device.device_type
    );
    let declared = location_of(device);
    let actual = route.route_location()?;
    ensure!(
        declared == actual,
        "device {} declares location {declared:?} but route {route_name} is {actual:?}",
        device.name
    );
    Ok(BoundDevice {
        name: device.name.clone(),
        kind: device.device_type.clone(),
        route: route_name,
        location: actual,
        clearance: device
            .generic("clearance")
            .and_then(|value| value.as_ident())
            .and_then(parse_class),
    })
}

/// Read a declassifier the checker has validated (E219).
fn declassifier(device: &awhdl_ast::DeviceDecl) -> Result<Declassifier> {
    let ident = |key: &str| device.generic(key).and_then(|value| value.as_ident());
    let class = |key: &str| {
        ident(key)
            .and_then(parse_class)
            .with_context(|| format!("declassifier {} needs {key}", device.name))
    };
    let filter = match (ident("filter"), ident("filter_method")) {
        (Some(filter), Some(method)) => Some((filter.to_owned(), method.to_owned())),
        (None, None) => None,
        _ => bail!(
            "declassifier {} needs filter and filter_method together",
            device.name
        ),
    };
    let approval = ident("approval") == Some("human");
    ensure!(
        filter.is_some() || approval,
        "declassifier {} has no means",
        device.name
    );
    Ok(Declassifier {
        name: device.name.clone(),
        from: class("from")?,
        to: class("to")?,
        filter,
        approval,
    })
}

fn constant(expr: &Expr) -> Result<Json> {
    ensure!(expr.names().is_empty(), "initial values must be constants");
    evaluate(expr, &|_| Ok(Json::Null))
}

/// Executes one device call. Production uses the engine; tests use a fake.
#[async_trait(?Send)]
pub trait Dispatcher {
    async fn call(
        &mut self,
        device: &BoundDevice,
        method: &str,
        arguments: &[Json],
    ) -> Result<Json>;
    /// Ask a human to release exactly this content. Without a trusted human
    /// channel a release fails closed.
    async fn approve_release(&mut self, request: &ReleaseRequest) -> Result<bool> {
        bail!(
            "no human approver is configured; declassifier {} cannot be approved",
            request.declassifier
        )
    }
    /// Metadata-only audit hook.
    fn record(&mut self, _event: &str, _data: Json) -> Result<()> {
        Ok(())
    }
    /// Refuse a new delta cycle once the caller's budget is exhausted.
    fn delta(&mut self) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DesignStatus {
    /// No pending events and no timers: the design finished.
    Quiescent,
    /// A budget ended a timer-driven or looping design.
    Exhausted,
}

#[derive(Debug, Clone, Serialize)]
pub struct DesignOutcome {
    pub status: DesignStatus,
    pub reason: Option<String>,
    pub outputs: BTreeMap<String, Json>,
    pub delta_cycles: u32,
    pub device_calls: u32,
}

#[derive(Default)]
struct DeltaWrites {
    signals: Vec<(String, Json, usize)>,
    events: BTreeSet<String>,
}

pub struct DesignRun<'a, D: Dispatcher> {
    design: &'a CompiledDesign,
    dispatcher: D,
    values: BTreeMap<String, Json>,
    /// Last call outcome per device, readable as `dev.done` / `dev.failed`.
    device_status: BTreeMap<String, bool>,
    /// Events of the current delta, readable as booleans (timers, barriers).
    current: BTreeSet<String>,
    barrier_seen: BTreeMap<String, BTreeSet<String>>,
    device_calls: u32,
}

impl<'a, D: Dispatcher> DesignRun<'a, D> {
    pub fn new(design: &'a CompiledDesign, dispatcher: D) -> Self {
        Self {
            values: design
                .signals
                .iter()
                .map(|(name, info)| (name.clone(), info.initial.clone()))
                .collect(),
            design,
            dispatcher,
            device_status: BTreeMap::new(),
            current: BTreeSet::new(),
            barrier_seen: BTreeMap::new(),
            device_calls: 0,
        }
    }

    pub fn into_dispatcher(self) -> D {
        self.dispatcher
    }

    pub async fn run(&mut self, inputs: BTreeMap<String, Json>) -> Result<DesignOutcome> {
        let started = Instant::now();
        let mut events = BTreeSet::new();
        for (name, value) in inputs {
            let info = self
                .design
                .signals
                .get(&name)
                .with_context(|| format!("unknown input {name}"))?;
            ensure!(
                info.port == Some(PortMode::In) || info.port == Some(PortMode::Inout),
                "{name} is not an input port"
            );
            self.values.insert(name.clone(), value);
            events.insert(name.clone());
            events.insert(format!("{name}.changed"));
        }
        let mut next_tick = self
            .design
            .timers
            .iter()
            .map(|(name, period)| (name.clone(), Duration::from_millis(*period)))
            .collect::<BTreeMap<_, _>>();
        let mut deltas = 0u32;
        let exhausted = |deltas: u32, reason: String, run: &Self| DesignOutcome {
            status: DesignStatus::Exhausted,
            reason: Some(reason),
            outputs: run.outputs(),
            delta_cycles: deltas,
            device_calls: run.device_calls,
        };
        loop {
            if let Some(limit) = self.design.limits.wall_time_ms
                && started.elapsed() >= Duration::from_millis(limit)
            {
                return Ok(exhausted(deltas, "wall_time budget".to_owned(), self));
            }
            if events.is_empty() {
                let Some((_, due)) = next_tick.iter().min_by_key(|(_, due)| **due) else {
                    break;
                };
                let due = *due;
                let wait = due.saturating_sub(started.elapsed());
                if let Some(limit) = self.design.limits.wall_time_ms
                    && due >= Duration::from_millis(limit)
                {
                    tokio::time::sleep(
                        Duration::from_millis(limit).saturating_sub(started.elapsed()),
                    )
                    .await;
                    return Ok(exhausted(deltas, "wall_time budget".to_owned(), self));
                }
                tokio::time::sleep(wait).await;
                for (name, tick) in next_tick.iter_mut() {
                    if *tick == due {
                        events.insert(name.clone());
                        *tick += Duration::from_millis(self.design.timers[name]);
                    }
                }
            }
            if self
                .design
                .limits
                .iterations
                .is_some_and(|limit| deltas >= limit)
            {
                return Ok(exhausted(deltas, "iterations budget".to_owned(), self));
            }
            if let Err(error) = self.dispatcher.delta() {
                return Ok(exhausted(deltas, format!("{error:#}"), self));
            }
            deltas += 1;
            self.dispatcher
                .record("design_delta", json!({"delta": deltas, "events": events}))?;
            events = self.delta(events).await?;
        }
        Ok(DesignOutcome {
            status: DesignStatus::Quiescent,
            reason: None,
            outputs: self.outputs(),
            delta_cycles: deltas,
            device_calls: self.device_calls,
        })
    }

    fn outputs(&self) -> BTreeMap<String, Json> {
        self.design
            .signals
            .iter()
            .filter(|(_, info)| matches!(info.port, Some(PortMode::Out | PortMode::Inout)))
            .map(|(name, _)| (name.clone(), self.values[name].clone()))
            .collect()
    }

    /// One delta: run triggered processes against a snapshot, then apply writes.
    async fn delta(&mut self, events: BTreeSet<String>) -> Result<BTreeSet<String>> {
        self.current = events;
        let mut writes = DeltaWrites::default();
        let design = self.design;
        for (index, process) in design.processes.iter().enumerate() {
            if !process
                .sensitivity
                .iter()
                .any(|name| self.current.contains(name))
            {
                continue;
            }
            let mut local = DeltaWrites::default();
            let outcome = match process.timeout {
                Some(limit) => {
                    match tokio::time::timeout(
                        Duration::from_millis(limit.millis),
                        self.block(&process.statements, index, &mut local),
                    )
                    .await
                    {
                        Ok(result) => result,
                        Err(_) => {
                            // The body's partial writes are discarded; only `on timeout` applies.
                            self.dispatcher
                                .record("process_timeout", json!({"process": index}))?;
                            local = DeltaWrites::default();
                            Box::pin(self.block(&process.on_timeout, index, &mut local)).await
                        }
                    }
                }
                None => self.block(&process.statements, index, &mut local).await,
            };
            outcome?;
            writes.signals.extend(local.signals);
            writes.events.extend(local.events);
        }
        let mut next = writes.events;
        let mut driven: BTreeMap<String, (Json, usize)> = BTreeMap::new();
        for (name, value, process) in writes.signals {
            if let Some((previous, driver)) = driven.get(&name) {
                ensure!(
                    *driver == process || *previous == value,
                    "signal {name} has conflicting drivers in one delta cycle"
                );
            }
            driven.insert(name, (value, process));
        }
        for (name, (value, _)) in driven {
            let changed = self.values.get(&name) != Some(&value);
            self.values.insert(name.clone(), value);
            self.dispatcher.record(
                "signal_updated",
                json!({"signal": name, "class": self.design.signals[&name].class, "changed": changed}),
            )?;
            if changed {
                next.insert(name.clone());
                next.insert(format!("{name}.changed"));
            }
            for (barrier, members) in &self.design.barriers {
                if members.contains(&name) {
                    let seen = self.barrier_seen.entry(barrier.clone()).or_default();
                    seen.insert(name.clone());
                    if seen.len() == members.len() {
                        seen.clear();
                        next.insert(format!("{barrier}.ready"));
                        next.insert(barrier.clone());
                    }
                }
            }
        }
        self.current.clear();
        for assertion in &self.design.assertions {
            let (condition, expected) = match assertion {
                Assertion::Always { condition } => (condition, true),
                Assertion::Never { condition } => (condition, false),
                Assertion::NeverFlow { .. } => continue,
            };
            let holds = self.eval_bool(condition)?;
            ensure!(
                holds == expected,
                "assertion violated: {} ({})",
                condition.source,
                if expected { "always" } else { "never" }
            );
        }
        Ok(next)
    }

    fn block<'s>(
        &'s mut self,
        statements: &'s [SequentialStatement],
        process: usize,
        writes: &'s mut DeltaWrites,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + 's>>
    where
        D: 's,
    {
        Box::pin(async move {
            for statement in statements {
                match statement {
                    SequentialStatement::Null { .. } => {}
                    SequentialStatement::Assignment(assignment) => {
                        let value = self.eval(&assignment.value)?;
                        writes
                            .signals
                            .push((assignment.target.clone(), value, process));
                    }
                    SequentialStatement::Assert { condition, .. } => {
                        ensure!(
                            self.eval_bool(condition)?,
                            "assertion failed: {}",
                            condition.source
                        );
                    }
                    SequentialStatement::If(branching) => {
                        let mut chosen = &branching.otherwise;
                        for branch in &branching.branches {
                            if self.eval_bool(&branch.condition)? {
                                chosen = &branch.statements;
                                break;
                            }
                        }
                        self.block(chosen, process, writes).await?;
                    }
                    SequentialStatement::DeviceCall(call) => {
                        self.call(call, process, writes).await?;
                    }
                    SequentialStatement::Declassify(release) => {
                        self.declassify(release, process, writes).await?;
                    }
                    // Logical parallelism: results commit together at the end of the
                    // delta. The v0.1 runtime dispatches the calls one after another.
                    SequentialStatement::Parallel(parallel) => {
                        for call in &parallel.calls {
                            self.call(call, process, writes).await?;
                        }
                    }
                }
            }
            Ok(())
        })
    }

    /// Release a value under a declassifier: the filter first, then the human;
    /// anything but an explicit pass and grant writes nothing.
    async fn declassify(
        &mut self,
        release: &awhdl_ast::Declassify,
        process: usize,
        writes: &mut DeltaWrites,
    ) -> Result<()> {
        let declassifier = self
            .design
            .declassifiers
            .get(&release.declassifier)
            .with_context(|| format!("unknown declassifier {}", release.declassifier))?
            .clone();
        let content = self.eval(&release.value)?;
        let content_sha256 = crate::effect::sha256_hex(&serde_json::to_vec(&content)?);
        let mut checks = serde_json::Map::new();
        let mut allowed = true;
        if let Some((filter, method)) = &declassifier.filter {
            let device = self
                .design
                .devices
                .get(filter)
                .with_context(|| format!("unbound filter device {filter}"))?
                .clone();
            self.device_calls += 1;
            self.dispatcher.record(
                "design_device_call",
                json!({"device": device.name, "route": device.route, "method": method, "class": self.class_of(&release.value.expr)}),
            )?;
            let verdict = self
                .dispatcher
                .call(&device, method, std::slice::from_ref(&content))
                .await;
            let pass = matches!(&verdict, Ok(Json::Object(map)) if map.get("pass") == Some(&Json::Bool(true)));
            checks.insert(
                "filter".to_owned(),
                json!(if pass { "pass" } else { "fail" }),
            );
            allowed = pass;
        }
        if allowed && declassifier.approval {
            let request = ReleaseRequest {
                declassifier: declassifier.name.clone(),
                from: declassifier.from,
                to: declassifier.to,
                source: release.value.source.clone(),
                target: release.target.clone(),
                context: format!(
                    "process({}): {} <= declassify {} using {}",
                    self.design.processes[process].sensitivity.join(", "),
                    release.target,
                    release.value.source,
                    release.declassifier
                ),
                content: content.clone(),
                content_sha256: content_sha256.clone(),
            };
            let decision = self.dispatcher.approve_release(&request).await;
            let granted = matches!(decision, Ok(true));
            checks.insert(
                "approval".to_owned(),
                json!(match decision {
                    Ok(true) => "granted".to_owned(),
                    Ok(false) => "denied".to_owned(),
                    Err(error) => format!("error: {error:#}"),
                }),
            );
            allowed = granted;
        }
        self.dispatcher.record(
            if allowed {
                "design_declassified"
            } else {
                "design_declassify_denied"
            },
            json!({
                "declassifier": declassifier.name,
                "from": declassifier.from,
                "to": declassifier.to,
                "source": release.value.source,
                "target": release.target,
                "content_sha256": content_sha256,
                "checks": checks,
            }),
        )?;
        self.device_status
            .insert(declassifier.name.clone(), allowed);
        if allowed {
            writes.events.insert(format!("{}.done", declassifier.name));
            writes
                .signals
                .push((release.target.clone(), content, process));
        } else {
            writes
                .events
                .insert(format!("{}.failed", declassifier.name));
        }
        Ok(())
    }

    async fn call(
        &mut self,
        call: &DeviceCall,
        process: usize,
        writes: &mut DeltaWrites,
    ) -> Result<()> {
        let device = self
            .design
            .devices
            .get(&call.device)
            .with_context(|| format!("unbound device {}", call.device))?
            .clone();
        let mut class = Class::Public;
        let mut arguments = Vec::new();
        for argument in &call.arguments {
            class = class.max(self.class_of(&argument.expr));
            arguments.push(self.eval(argument)?);
        }
        // Runtime egress guard, independent of the static check.
        ensure!(
            !(device.location == Location::Cloud && class >= self.design.cloud_floor),
            "cloud egress denied: {class:?} data to device {}",
            device.name
        );
        if let Some(clearance) = device.clearance {
            ensure!(
                class <= clearance,
                "{class:?} data exceeds {clearance:?} clearance of device {}",
                device.name
            );
        }
        self.device_calls += 1;
        self.dispatcher.record(
            "design_device_call",
            json!({"device": device.name, "route": device.route, "method": call.method, "class": class}),
        )?;
        let pending = self.dispatcher.call(&device, &call.method, &arguments);
        let result = match call.timeout {
            Some(limit) => {
                match tokio::time::timeout(Duration::from_millis(limit.millis), pending).await {
                    Ok(result) => result,
                    Err(_) => {
                        writes.events.insert(format!("{}.timeout", device.name));
                        Err(anyhow::anyhow!("device call timed out"))
                    }
                }
            }
            None => pending.await,
        };
        match result {
            Ok(value) => {
                self.device_status.insert(device.name.clone(), true);
                writes.events.insert(format!("{}.done", device.name));
                writes.signals.push((call.output.clone(), value, process));
            }
            Err(error) => {
                self.device_status.insert(device.name.clone(), false);
                writes.events.insert(format!("{}.failed", device.name));
                self.dispatcher.record(
                    "design_device_failed",
                    json!({"device": device.name, "error": format!("{error:#}")}),
                )?;
            }
        }
        Ok(())
    }

    fn class_of(&self, expr: &Expr) -> Class {
        expr.names()
            .into_iter()
            .filter_map(|path| self.design.signals.get(&path[0]).map(|info| info.class))
            .max()
            .unwrap_or(Class::Public)
    }

    fn eval(&self, expression: &Expression) -> Result<Json> {
        evaluate(&expression.expr, &|path| self.lookup(path))
            .with_context(|| format!("evaluating {}", expression.source))
    }

    fn eval_bool(&self, expression: &Expression) -> Result<bool> {
        self.eval(expression)?
            .as_bool()
            .with_context(|| format!("condition {} is not boolean", expression.source))
    }

    fn lookup(&self, path: &[String]) -> Result<Json> {
        let root = path[0].as_str();
        if let Some(value) = self.values.get(root) {
            let mut value = value;
            for field in &path[1..] {
                value = value
                    .get(field)
                    .with_context(|| format!("{} has no field {field}", path.join(".")))?;
            }
            return Ok(value.clone());
        }
        if self.design.devices.contains_key(root) {
            let status = self.device_status.get(root).copied();
            return Ok(match path.get(1).map(String::as_str) {
                Some("done") => Json::Bool(status == Some(true)),
                Some("failed") => Json::Bool(status == Some(false)),
                _ => bail!("device {root} exposes only .done and .failed"),
            });
        }
        // Timers and barriers read as whether they fired in this delta.
        Ok(Json::Bool(self.current.contains(&path.join("."))))
    }
}

fn evaluate(expr: &Expr, lookup: &dyn Fn(&[String]) -> Result<Json>) -> Result<Json> {
    Ok(match expr {
        Expr::String { value } => json!(value),
        Expr::Integer { value } => json!(value),
        Expr::Bool { value } => json!(value),
        Expr::Time { value } => json!(value.millis),
        Expr::Name { path } => lookup(path)?,
        Expr::Not { operand } => json!(
            !evaluate(operand, lookup)?
                .as_bool()
                .context("not needs a boolean")?
        ),
        Expr::Binary { op, left, right } => {
            let left = evaluate(left, lookup)?;
            match op {
                BinaryOp::And | BinaryOp::Or => {
                    let left = left.as_bool().context("and/or need booleans")?;
                    if (*op == BinaryOp::And && !left) || (*op == BinaryOp::Or && left) {
                        return Ok(json!(left));
                    }
                    json!(
                        evaluate(right, lookup)?
                            .as_bool()
                            .context("and/or need booleans")?
                    )
                }
                BinaryOp::Eq => json!(left == evaluate(right, lookup)?),
                BinaryOp::Ne => json!(left != evaluate(right, lookup)?),
                BinaryOp::Add | BinaryOp::Sub => {
                    let right = evaluate(right, lookup)?;
                    let (a, b) = (
                        left.as_i64().context("arithmetic needs integers")?,
                        right.as_i64().context("arithmetic needs integers")?,
                    );
                    json!(
                        if *op == BinaryOp::Add {
                            a.checked_add(b)
                        } else {
                            a.checked_sub(b)
                        }
                        .context("integer overflow")?
                    )
                }
                BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                    let right = evaluate(right, lookup)?;
                    let ordering = match (&left, &right) {
                        (Json::Number(a), Json::Number(b)) => a
                            .as_f64()
                            .zip(b.as_f64())
                            .and_then(|(a, b)| a.partial_cmp(&b)),
                        (Json::String(a), Json::String(b)) => Some(a.cmp(b)),
                        _ => None,
                    }
                    .context("ordering needs two numbers or two strings")?;
                    json!(match op {
                        BinaryOp::Lt => ordering.is_lt(),
                        BinaryOp::Le => ordering.is_le(),
                        BinaryOp::Gt => ordering.is_gt(),
                        _ => ordering.is_ge(),
                    })
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> ProjectConfig {
        ProjectConfig::load(crate::test_support::project_root()).unwrap()
    }

    type Respond = Box<dyn Fn(&str, &str, &[Json]) -> Result<Json>>;

    struct Fake {
        calls: Vec<(String, String, Vec<Json>)>,
        respond: Respond,
        delay: Duration,
        /// `None`: no human approver (the trait default fails closed).
        approve: Option<bool>,
        asked: Vec<ReleaseRequest>,
    }

    fn fake(respond: impl Fn(&str, &str, &[Json]) -> Result<Json> + 'static) -> Fake {
        Fake {
            calls: Vec::new(),
            respond: Box::new(respond),
            delay: Duration::ZERO,
            approve: None,
            asked: Vec::new(),
        }
    }

    #[async_trait(?Send)]
    impl Dispatcher for Fake {
        async fn call(
            &mut self,
            device: &BoundDevice,
            method: &str,
            arguments: &[Json],
        ) -> Result<Json> {
            self.calls
                .push((device.name.clone(), method.to_owned(), arguments.to_vec()));
            tokio::time::sleep(self.delay).await;
            (self.respond)(&device.name, method, arguments)
        }

        async fn approve_release(&mut self, request: &ReleaseRequest) -> Result<bool> {
            self.asked.push(request.clone());
            match self.approve {
                Some(granted) => Ok(granted),
                None => bail!("no human approver"),
            }
        }
    }

    const HEADER: &str = r#"
entity work is
    port (
        task   : in  text<internal>;
        report : out text<internal>
    );
end work;
architecture flow of work is
    device coder : agent generic (route => "initial_coding");
    device matlab : mcp generic (route => "calculation_graphing");
    device reasoner : agent generic (location => cloud, route => "deep_reasoning");
    signal candidate : code<internal>;
    signal value : number<internal> := 0;
    signal tries : number<internal> := 0;
    DECLS
begin
    BODY
end flow;
"#;

    fn design(decls: &str, body: &str) -> Result<CompiledDesign> {
        compile(
            &HEADER.replace("DECLS", decls).replace("BODY", body),
            &config(),
        )
    }

    const NOOP: &str = "process(task) begin null; end process;";

    fn task(text: &str) -> BTreeMap<String, Json> {
        BTreeMap::from([("task".to_owned(), json!(text))])
    }

    #[tokio::test]
    async fn pipeline_runs_processes_in_delta_cycles() {
        let design = design(
            "",
            r#"
    process(task)
    begin
        coder.run(task) -> candidate;
    end process;
    process(candidate)
    begin
        matlab.evaluate_matlab_code(candidate) -> value;
    end process;
    process(value.changed)
    begin
        if value > 50 then
            report <= "large";
        else
            report <= "small";
        end if;
    end process;
"#,
        )
        .unwrap();
        let mut run = DesignRun::new(
            &design,
            fake(|device, _, arguments| match device {
                "coder" => Ok(json!(format!("x = {}", arguments[0].as_str().unwrap()))),
                "matlab" => Ok(json!(55)),
                other => bail!("unexpected {other}"),
            }),
        );
        let outcome = run.run(task("sum")).await.unwrap();
        assert_eq!(outcome.status, DesignStatus::Quiescent);
        assert_eq!(outcome.outputs["report"], json!("large"));
        assert_eq!(outcome.delta_cycles, 4);
        let fake = run.into_dispatcher();
        assert_eq!(
            fake.calls[1],
            (
                "matlab".to_owned(),
                "evaluate_matlab_code".to_owned(),
                vec![json!("x = sum")]
            )
        );
    }

    #[tokio::test]
    async fn feedback_loop_stops_by_condition_and_assertions_hold() {
        let design = design(
            "",
            r#"
    assert always (tries <= 3);
    process(task, tries)
    begin
        if tries < 3 then
            tries <= tries + 1;
        else
            report <= "done";
        end if;
    end process;
"#,
        )
        .unwrap();
        let outcome = DesignRun::new(&design, fake(|_, _, _| bail!("no calls")))
            .run(task("go"))
            .await
            .unwrap();
        assert_eq!(outcome.outputs["report"], json!("done"));
        // A bound that the loop would cross is an assertion failure, not a silent stop.
        let violating = design_with_bound(2);
        let error = DesignRun::new(&violating, fake(|_, _, _| bail!("no calls")))
            .run(task("go"))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("assertion violated"));
    }

    fn design_with_bound(bound: i64) -> CompiledDesign {
        design(
            "",
            &format!(
                r#"
    assert always (tries <= {bound});
    process(task, tries)
    begin
        if tries < 3 then
            tries <= tries + 1;
        end if;
    end process;
"#
            ),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn timers_need_budgets_and_budgets_end_the_run() {
        assert!(
            format!(
                "{:#}",
                design("timer tick : period 5 ms;", NOOP).unwrap_err()
            )
            .contains("needs an iterations")
        );
        let design = design(
            "timer tick : period 5 ms;\n    budget b is iterations <= 4; end budget;",
            r#"
    process(tick)
    begin
        tries <= tries + 1;
    end process;
"#,
        )
        .unwrap();
        let outcome = DesignRun::new(&design, fake(|_, _, _| bail!("no calls")))
            .run(BTreeMap::new())
            .await
            .unwrap();
        assert_eq!(outcome.status, DesignStatus::Exhausted);
        assert_eq!(outcome.delta_cycles, 4);
        let wall = super::tests::design(
            "timer tick : period 5 ms;\n    budget b is wall_time <= 30 ms; end budget;",
            "process(tick) begin null; end process;",
        )
        .unwrap();
        let started = Instant::now();
        let outcome = DesignRun::new(&wall, fake(|_, _, _| bail!("no calls")))
            .run(BTreeMap::new())
            .await
            .unwrap();
        assert_eq!(outcome.status, DesignStatus::Exhausted);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    const RELEASE: &str = r#"
    device pii : deterministic generic (route => "text_filtering");
    device release : declassifier generic (from => restricted, to => internal,
        filter => pii, filter_method => scan, approval => human);
    signal secret : text<restricted>;
    signal released : text<internal>;"#;

    const RELEASE_BODY: &str = r#"
    process(task)
    begin
        secret <= task;
    end process;
    process(secret)
    begin
        released <= declassify secret using release;
    end process;
    process(released)
    begin
        report <= released;
    end process;
    process(release.failed)
    begin
        secret <= "refused";
    end process;
"#;

    async fn release_run(filter: Json, approve: Option<bool>) -> (DesignOutcome, Fake) {
        let design = design(RELEASE, RELEASE_BODY).unwrap();
        let mut fake = fake(move |_, _, _| Ok(filter.clone()));
        fake.approve = approve;
        let mut run = DesignRun::new(&design, fake);
        let outcome = run.run(task("draft")).await.unwrap();
        (outcome, run.into_dispatcher())
    }

    #[tokio::test]
    async fn declassify_releases_only_after_the_filter_and_the_human_allow_it() {
        // Filter pass and human grant: the content is released as internal.
        let (outcome, fake) = release_run(json!({"pass": true}), Some(true)).await;
        assert_eq!(outcome.outputs["report"], json!("draft"));
        assert_eq!(fake.calls.len(), 1);
        assert_eq!(fake.calls[0].1, "scan");
        let asked = &fake.asked[0];
        assert_eq!(asked.content, json!("draft"));
        assert!(
            asked
                .context
                .starts_with("process(secret): released <= declassify secret")
        );

        // Filter fail: the human is never asked and nothing is released.
        let (outcome, fake) = release_run(json!({"pass": false}), Some(true)).await;
        assert_eq!(outcome.outputs["report"], Json::Null);
        assert!(fake.asked.is_empty());
        // Anything but a boolean `pass: true` fails.
        let (outcome, _) = release_run(json!({"pass": "yes"}), Some(true)).await;
        assert_eq!(outcome.outputs["report"], Json::Null);

        // Human denial, and no approver at all, both fail closed.
        let (outcome, fake) = release_run(json!({"pass": true}), Some(false)).await;
        assert_eq!(outcome.outputs["report"], Json::Null);
        // The failure handler rewrites the secret, so the human is asked again
        // about the new content, and denies again.
        let contents = fake
            .asked
            .iter()
            .map(|asked| &asked.content)
            .collect::<Vec<_>>();
        assert_eq!(contents, [&json!("draft"), &json!("refused")]);
        let (outcome, _) = release_run(json!({"pass": true}), None).await;
        assert_eq!(outcome.outputs["report"], Json::Null);
    }

    #[tokio::test]
    async fn timeouts_fail_calls_and_run_on_timeout_handlers() {
        let design = design(
            "",
            r#"
    process(task)
    begin
        coder.run(task) timeout 10 ms -> candidate;
    end process;
    process(coder.failed)
    begin
        report <= "call failed";
    end process;
"#,
        )
        .unwrap();
        let mut slow = fake(|_, _, _| Ok(json!("late")));
        slow.delay = Duration::from_millis(200);
        let outcome = DesignRun::new(&design, slow).run(task("x")).await.unwrap();
        assert_eq!(outcome.outputs["report"], json!("call failed"));

        let design = super::tests::design(
            "",
            r#"
    process(task) timeout 10 ms;
    begin
        report <= "partial";
        coder.run(task) -> candidate;
    on timeout
        report <= "process timed out";
    end process;
"#,
        )
        .unwrap();
        let mut slow = fake(|_, _, _| Ok(json!("late")));
        slow.delay = Duration::from_millis(200);
        let outcome = DesignRun::new(&design, slow).run(task("x")).await.unwrap();
        assert_eq!(outcome.outputs["report"], json!("process timed out"));
    }

    #[tokio::test]
    async fn a_call_timeout_wakes_processes_sensitive_to_device_timeout() {
        let design = design(
            "",
            r#"
    process(task)
    begin
        coder.run(task) timeout 10 ms -> candidate;
    end process;
    process(coder.timeout)
    begin
        report <= "call timed out";
    end process;
"#,
        )
        .unwrap();
        let mut slow = fake(|_, _, _| Ok(json!("late")));
        slow.delay = Duration::from_millis(200);
        let outcome = DesignRun::new(&design, slow).run(task("x")).await.unwrap();
        assert_eq!(outcome.outputs["report"], json!("call timed out"));
    }

    #[tokio::test]
    async fn parallel_results_commit_together_and_barrier_fires() {
        let design = design(
            "signal left_result, right_result : number<internal>;\n    barrier both (left_result, right_result);",
            r#"
    process(task)
    begin
        parallel
            matlab.evaluate_matlab_code(task) -> left_result;
            coder.run(task) -> right_result;
        end parallel;
    end process;
    process(both.ready)
    begin
        report <= "both ready";
    end process;
"#,
        )
        .unwrap();
        let outcome = DesignRun::new(&design, fake(|device, _, _| Ok(json!(device.len()))))
            .run(task("x"))
            .await
            .unwrap();
        assert_eq!(outcome.outputs["report"], json!("both ready"));
        assert_eq!(outcome.device_calls, 2);
    }

    #[tokio::test]
    async fn cloud_egress_is_checked_statically_and_again_at_runtime() {
        let restricted = HEADER.replace(
            "task   : in  text<internal>",
            "task   : in  text<restricted>",
        );
        let body = "process(task) begin reasoner.run(task) -> report; end process;";
        let error = compile(
            &restricted.replace("DECLS", "").replace("BODY", body),
            &config(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("AWHDL-E301"));
        // If a class were wrong after compilation, the runtime guard still refuses.
        let mut design = design("", body).unwrap();
        design.signals.get_mut("task").unwrap().class = Class::Restricted;
        let mut run = DesignRun::new(&design, fake(|_, _, _| Ok(json!("sent"))));
        let error = run.run(task("secret")).await.unwrap_err();
        assert!(format!("{error:#}").contains("cloud egress denied"));
        assert!(run.into_dispatcher().calls.is_empty());
    }

    #[test]
    fn binding_checks_kind_location_route_and_budget_support() {
        let hello = include_str!("../../../examples/hello.awhdl");
        assert!(compile(hello, &config()).is_err());
        let local_codex = HEADER.replace("location => cloud, ", "");
        let error = compile(
            &local_codex.replace("DECLS", "").replace("BODY", NOOP),
            &config(),
        )
        .unwrap_err();
        assert!(
            format!("{error:#}").contains("route deep_reasoning is Cloud"),
            "{error:#}"
        );
        let wrong_kind = HEADER.replace("device matlab : mcp", "device matlab : agent");
        assert!(
            compile(
                &wrong_kind.replace("DECLS", "").replace("BODY", NOOP),
                &config()
            )
            .is_err()
        );
        let unknown = HEADER.replace("\"calculation_graphing\"", "\"nope\"");
        assert!(
            compile(
                &unknown.replace("DECLS", "").replace("BODY", NOOP),
                &config()
            )
            .is_err()
        );
        assert!(design("budget b is llm_tokens <= 10; end budget;", NOOP).is_err());
    }

    #[tokio::test]
    async fn conflicting_drivers_in_one_delta_are_rejected() {
        let design = design(
            "",
            r#"
    process(task) begin report <= "a"; end process;
    process(task) begin report <= "b"; end process;
"#,
        )
        .unwrap();
        let error = DesignRun::new(&design, fake(|_, _, _| bail!("no calls")))
            .run(task("x"))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("conflicting drivers"));
    }
}
