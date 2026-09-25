//! Static checks. Structural errors are `AWHDL-E2xx`, information-flow errors
//! are `AWHDL-E3xx`, and constructs outside the v0.1 profile are `AWHDL-E4xx`.
use awhdl_ast::{
    ArchitectureDecl, Assertion, BudgetValue, ConcurrentStatement, DataType, Declaration, Design,
    DesignUnit, DeviceCall, DeviceDecl, Expr, Expression, PortMode, ProcessStmt,
    SequentialStatement, Span,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    pub code: &'static str,
    pub message: String,
    pub span: Span,
}

/// Security classes admitted by the v0.1 profile, in increasing order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    Public,
    Internal,
    Restricted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Location {
    Local,
    Cloud,
}

/// Classes and locations that the language defines but v0.1 does not implement.
const DEFERRED_CLASSES: [&str; 2] = ["confidential", "secret"];
const DEFERRED_LOCATIONS: [&str; 3] = ["sandbox", "private_cloud", "external"];
const COUNT_LIMITS: [&str; 5] = [
    "iterations",
    "tool_calls",
    "llm_tokens",
    "network_requests",
    "model_calls",
];
const TIME_LIMITS: [&str; 1] = ["wall_time"];

pub fn parse_class(name: &str) -> Option<Class> {
    match name {
        "public" => Some(Class::Public),
        "internal" => Some(Class::Internal),
        "restricted" => Some(Class::Restricted),
        _ => None,
    }
}

pub fn parse_location(name: &str) -> Option<Location> {
    match name {
        "local" => Some(Location::Local),
        "cloud" => Some(Location::Cloud),
        _ => None,
    }
}

/// An unlabeled type is treated as `restricted`: data is never assumed safe.
pub fn class_of(data_type: &DataType) -> Class {
    data_type
        .classification
        .as_deref()
        .and_then(parse_class)
        .unwrap_or(Class::Restricted)
}

/// A device's location; an unspecified location is local.
pub fn location_of(device: &DeviceDecl) -> Location {
    device
        .generic("location")
        .and_then(|value| value.as_ident())
        .and_then(parse_location)
        .unwrap_or(Location::Local)
}

pub fn check(design: &Design) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut entities = BTreeMap::new();

    for unit in &design.units {
        if let DesignUnit::Entity(entity) = unit {
            if entities.insert(entity.name.as_str(), entity).is_some() {
                diagnostics.push(diag(
                    "AWHDL-E201",
                    format!("duplicate entity declaration: {}", entity.name),
                    entity.span,
                ));
            }
            let mut port_names = BTreeSet::new();
            for port in &entity.ports {
                label(&port.data_type, port.span, &mut diagnostics);
                for name in &port.names {
                    if !port_names.insert(name) {
                        diagnostics.push(diag(
                            "AWHDL-E202",
                            format!("duplicate port declaration: {name}"),
                            port.span,
                        ));
                    }
                }
            }
        }
    }

    for unit in &design.units {
        let DesignUnit::Architecture(architecture) = unit else {
            continue;
        };
        let Some(entity) = entities.get(architecture.entity.as_str()) else {
            diagnostics.push(diag(
                "AWHDL-E203",
                format!(
                    "architecture {} references unknown entity {}",
                    architecture.name, architecture.entity
                ),
                architecture.span,
            ));
            continue;
        };
        let mut scope = Scope::default();
        for port in &entity.ports {
            for name in &port.names {
                scope.values.insert(
                    name.clone(),
                    Value {
                        class: class_of(&port.data_type),
                        input_port: port.mode == PortMode::In,
                    },
                );
            }
        }
        scope.declare(architecture, &mut diagnostics);
        scope.check_declassifiers(&mut diagnostics);
        scope.check_statements(architecture, &mut diagnostics);
    }
    diagnostics
}

fn diag(code: &'static str, message: String, span: Span) -> Diagnostic {
    Diagnostic {
        code,
        message,
        span,
    }
}

fn label(data_type: &DataType, span: Span, diagnostics: &mut Vec<Diagnostic>) {
    if let Some(name) = &data_type.classification {
        profile_name(
            name,
            parse_class(name).is_some(),
            &DEFERRED_CLASSES,
            "classification",
            span,
            diagnostics,
        );
    }
}

fn profile_name(
    name: &str,
    supported: bool,
    deferred: &[&str],
    what: &str,
    span: Span,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if supported {
        return;
    }
    diagnostics.push(if deferred.contains(&name) {
        diag(
            "AWHDL-E401",
            format!("{what} {name} is not in the v0.1 profile"),
            span,
        )
    } else {
        diag("AWHDL-E402", format!("unknown {what}: {name}"), span)
    });
}

/// The class a statement's execution reveals, and where it comes from.
#[derive(Clone)]
struct Context {
    class: Class,
    reason: String,
}

impl Context {
    fn public() -> Self {
        Context {
            class: Class::Public,
            reason: "public".to_owned(),
        }
    }

    fn raise(&mut self, class: Class, reason: impl FnOnce() -> String) {
        if class > self.class {
            self.class = class;
            self.reason = reason();
        }
    }
}

enum Site<'s> {
    Call(&'s DeviceCall, Context),
    Declassify(&'s awhdl_ast::Declassify, Context),
    Assign(&'s awhdl_ast::Assignment, Context),
}

struct Value {
    class: Class,
    input_port: bool,
}

#[derive(Default)]
struct Scope<'a> {
    values: BTreeMap<String, Value>,
    devices: BTreeMap<String, &'a DeviceDecl>,
    timers: BTreeSet<String>,
    barriers: BTreeSet<String>,
    /// Barrier name -> member signals.
    barrier_members: BTreeMap<String, Vec<String>>,
    budgets: BTreeSet<String>,
    /// Minimum class that may never reach a cloud device. `restricted` always.
    cloud_floor: Option<Class>,
}

impl<'a> Scope<'a> {
    fn taken(&self, name: &str) -> bool {
        self.values.contains_key(name)
            || self.devices.contains_key(name)
            || self.timers.contains(name)
            || self.barriers.contains(name)
            || self.budgets.contains(name)
    }

    fn declare(&mut self, architecture: &'a ArchitectureDecl, diagnostics: &mut Vec<Diagnostic>) {
        for declaration in &architecture.declarations {
            match declaration {
                Declaration::Device(device) => {
                    if self.taken(&device.name) {
                        diagnostics.push(diag(
                            "AWHDL-E204",
                            format!("duplicate device declaration: {}", device.name),
                            device.span,
                        ));
                    }
                    self.devices.insert(device.name.clone(), device);
                    check_device(device, diagnostics);
                }
                Declaration::Signal(signal) => {
                    label(&signal.data_type, signal.span, diagnostics);
                    for name in &signal.names {
                        if self.taken(name) {
                            diagnostics.push(diag(
                                "AWHDL-E205",
                                format!("duplicate declaration: {name}"),
                                signal.span,
                            ));
                        }
                        self.values.insert(
                            name.clone(),
                            Value {
                                class: class_of(&signal.data_type),
                                input_port: false,
                            },
                        );
                    }
                }
                Declaration::Timer(timer) => {
                    if self.taken(&timer.name) {
                        diagnostics.push(diag(
                            "AWHDL-E205",
                            format!("duplicate declaration: {}", timer.name),
                            timer.span,
                        ));
                    }
                    if timer.period.millis == 0 {
                        diagnostics.push(diag(
                            "AWHDL-E211",
                            format!("timer {} needs a positive period", timer.name),
                            timer.span,
                        ));
                    }
                    self.timers.insert(timer.name.clone());
                }
                Declaration::Budget(budget) => {
                    if self.taken(&budget.name) {
                        diagnostics.push(diag(
                            "AWHDL-E205",
                            format!("duplicate declaration: {}", budget.name),
                            budget.span,
                        ));
                    }
                    let mut keys = BTreeSet::new();
                    for limit in &budget.limits {
                        let valid = match limit.value {
                            BudgetValue::Count(count) => {
                                COUNT_LIMITS.contains(&limit.key.as_str()) && count > 0
                            }
                            BudgetValue::Time(time) => {
                                TIME_LIMITS.contains(&limit.key.as_str()) && time.millis > 0
                            }
                        };
                        if !valid || !keys.insert(&limit.key) {
                            diagnostics.push(diag(
                                "AWHDL-E212",
                                format!("invalid or duplicate budget limit: {}", limit.key),
                                limit.span,
                            ));
                        }
                    }
                    self.budgets.insert(budget.name.clone());
                }
                Declaration::Barrier(barrier) => {
                    if self.taken(&barrier.name) {
                        diagnostics.push(diag(
                            "AWHDL-E205",
                            format!("duplicate declaration: {}", barrier.name),
                            barrier.span,
                        ));
                    }
                    for member in &barrier.members {
                        if !self.values.contains_key(member) {
                            diagnostics.push(diag(
                                "AWHDL-E214",
                                format!(
                                    "barrier {} member is not a signal: {member}",
                                    barrier.name
                                ),
                                barrier.span,
                            ));
                        }
                    }
                    self.barriers.insert(barrier.name.clone());
                    self.barrier_members
                        .insert(barrier.name.clone(), barrier.members.clone());
                }
            }
        }
    }

    fn check_statements(
        &mut self,
        architecture: &ArchitectureDecl,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        self.cloud_floor = Some(Class::Restricted);
        for statement in &architecture.statements {
            if let ConcurrentStatement::Assert(assert) = statement {
                match &assert.assertion {
                    Assertion::NeverFlow {
                        classification,
                        location,
                    } => {
                        let class = parse_class(classification);
                        profile_name(
                            classification,
                            class.is_some(),
                            &DEFERRED_CLASSES,
                            "classification",
                            assert.span,
                            diagnostics,
                        );
                        let place = parse_location(location);
                        profile_name(
                            location,
                            place.is_some(),
                            &DEFERRED_LOCATIONS,
                            "location",
                            assert.span,
                            diagnostics,
                        );
                        if let (Some(class), Some(Location::Cloud)) = (class, place) {
                            self.cloud_floor = self.cloud_floor.map(|floor| floor.min(class));
                        }
                    }
                    Assertion::Always { condition } | Assertion::Never { condition } => {
                        self.check_expression(condition, diagnostics);
                    }
                }
            }
        }
        for statement in &architecture.statements {
            let ConcurrentStatement::Process(process) = statement else {
                continue;
            };
            for trigger in &process.sensitivity {
                if let Some(message) = self.sensitivity_error(trigger) {
                    diagnostics.push(diag("AWHDL-E206", message, process.span));
                }
            }
            if process.timeout.is_some_and(|time| time.millis == 0) {
                diagnostics.push(diag(
                    "AWHDL-E211",
                    "process timeout must be positive".to_owned(),
                    process.span,
                ));
            }
            for sequential in process.statements.iter().chain(&process.on_timeout) {
                self.check_sequential(sequential, diagnostics);
            }
        }
        self.check_implicit_flows(architecture, diagnostics);
    }

    // -----------------------------------------------------------------------
    // Implicit flows (SECURITY_SPEC): whether and when a statement runs
    // carries the class of what triggered it and of the conditions around it.

    fn check_implicit_flows(
        &self,
        architecture: &ArchitectureDecl,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let processes = architecture
            .statements
            .iter()
            .filter_map(|statement| match statement {
                ConcurrentStatement::Process(process) => Some(process),
                ConcurrentStatement::Assert(_) => None,
            })
            .collect::<Vec<_>>();
        // Device event classes depend on call contexts and vice versa: iterate
        // from public until nothing rises (a finite lattice, so this ends).
        let mut events: BTreeMap<&str, Class> = BTreeMap::new();
        let sites = loop {
            let mut sites = Vec::new();
            for process in &processes {
                let context = self.process_context(process, &events);
                self.sites(&process.statements, &context, &mut sites);
                let timed = self.timeout_context(&process.statements, &context);
                self.sites(&process.on_timeout, &timed, &mut sites);
            }
            let mut next: BTreeMap<&str, Class> = BTreeMap::new();
            for site in &sites {
                // A call's or a release's outcome depends on what it was given.
                let (device, class) = match site {
                    Site::Call(call, context) => (
                        call.device.as_str(),
                        self.call_class(call).max(context.class),
                    ),
                    Site::Declassify(release, context) => (
                        release.declassifier.as_str(),
                        self.class_of_expr(&release.value.expr).max(context.class),
                    ),
                    Site::Assign(..) => continue,
                };
                let entry = next.entry(device).or_insert(Class::Public);
                *entry = (*entry).max(class);
            }
            if next == events {
                break sites;
            }
            events = next;
        };
        for site in sites {
            match site {
                Site::Declassify(release, context) => {
                    let (Some(target), Some((from, to))) = (
                        self.values.get(&release.target),
                        self.declassifier_range(&release.declassifier),
                    ) else {
                        continue;
                    };
                    // A human who approves sees the release in its context, so the
                    // context may reach `from`; a filter sees only the content.
                    let approved = self
                        .devices
                        .get(&release.declassifier)
                        .and_then(|device| device.generic("approval"))
                        .is_some();
                    let allowed = if approved { from } else { target.class };
                    if to <= target.class && context.class > allowed {
                        diagnostics.push(diag(
                            "AWHDL-E304",
                            format!(
                                "implicit flow: {:?} context ({}) cannot write {:?} signal {}{}",
                                context.class,
                                context.reason,
                                target.class,
                                release.target,
                                if approved {
                                    ""
                                } else {
                                    "; a filter-only declassifier checks the content, not the context"
                                }
                            ),
                            release.span,
                        ));
                    }
                }
                Site::Assign(assignment, context) => {
                    let Some(target) = self.values.get(&assignment.target) else {
                        continue;
                    };
                    let explicit = self.class_of_expr(&assignment.value.expr);
                    if explicit <= target.class && context.class > target.class {
                        diagnostics.push(diag(
                            "AWHDL-E304",
                            format!(
                                "implicit flow: {:?} context ({}) cannot write {:?} signal {}",
                                context.class, context.reason, target.class, assignment.target
                            ),
                            assignment.span,
                        ));
                    }
                }
                Site::Call(call, context) => {
                    let explicit = self.call_class(call);
                    if let Some(output) = self.values.get(&call.output)
                        && explicit <= output.class
                        && context.class > output.class
                    {
                        diagnostics.push(diag(
                            "AWHDL-E304",
                            format!(
                                "implicit flow: {:?} context ({}) cannot write {:?} signal {} from {}.{}",
                                context.class,
                                context.reason,
                                output.class,
                                call.output,
                                call.device,
                                call.method
                            ),
                            call.span,
                        ));
                    }
                    let Some(device) = self.devices.get(&call.device) else {
                        continue;
                    };
                    if location_of(device) == Location::Cloud
                        && let Some(floor) = self.cloud_floor
                        && explicit < floor
                        && context.class >= floor
                    {
                        diagnostics.push(diag(
                            "AWHDL-E305",
                            format!(
                                "implicit flow: calling cloud device {} in a {:?} context ({}) reveals it",
                                call.device, context.class, context.reason
                            ),
                            call.span,
                        ));
                    }
                    if let Some(clearance) = device
                        .generic("clearance")
                        .and_then(|value| value.as_ident())
                        .and_then(parse_class)
                        && explicit <= clearance
                        && context.class > clearance
                    {
                        diagnostics.push(diag(
                            "AWHDL-E306",
                            format!(
                                "implicit flow: {:?} context ({}) exceeds {:?} clearance of device {}",
                                context.class, context.reason, clearance, call.device
                            ),
                            call.span,
                        ));
                    }
                }
            }
        }
    }

    /// The explicit class of a call: the strongest class its arguments read.
    fn call_class(&self, call: &DeviceCall) -> Class {
        call.arguments
            .iter()
            .map(|argument| self.class_of_expr(&argument.expr))
            .max()
            .unwrap_or(Class::Public)
    }

    /// The class of a sensitivity event (SECURITY_SPEC, event classes).
    fn event_class(&self, trigger: &str, devices: &BTreeMap<&str, Class>) -> Class {
        let (root, member) = match trigger.split_once('.') {
            Some((root, member)) => (root, Some(member)),
            None => (trigger, None),
        };
        if let Some(value) = self.values.get(root) {
            return value.class;
        }
        if self.barriers.contains(root) {
            return self
                .barrier_members
                .get(root)
                .into_iter()
                .flatten()
                .filter_map(|member| self.values.get(member).map(|value| value.class))
                .max()
                .unwrap_or(Class::Public);
        }
        if member.is_some() && self.devices.contains_key(root) {
            return devices.get(root).copied().unwrap_or(Class::Public);
        }
        Class::Public
    }

    fn process_context(&self, process: &ProcessStmt, devices: &BTreeMap<&str, Class>) -> Context {
        let mut context = Context::public();
        for trigger in &process.sensitivity {
            context.raise(self.event_class(trigger, devices), || {
                format!("triggered by {trigger}")
            });
        }
        context
    }

    /// `on timeout` runs depending on how long the body's calls take.
    fn timeout_context(&self, body: &[SequentialStatement], outer: &Context) -> Context {
        let mut sites = Vec::new();
        self.sites(body, outer, &mut sites);
        let mut context = outer.clone();
        for site in sites {
            match site {
                Site::Call(call, call_context) => {
                    context.raise(self.call_class(call).max(call_context.class), || {
                        format!(
                            "on timeout of a body calling {}.{}",
                            call.device, call.method
                        )
                    });
                }
                Site::Declassify(release, call_context) => {
                    let class = self
                        .class_of_expr(&release.value.expr)
                        .max(call_context.class);
                    context.raise(class, || {
                        format!("on timeout of a body using {}", release.declassifier)
                    });
                }
                Site::Assign(..) => {}
            }
        }
        context
    }

    /// Every call and assignment with the context it runs in.
    fn sites<'s>(
        &self,
        statements: &'s [SequentialStatement],
        context: &Context,
        sites: &mut Vec<Site<'s>>,
    ) {
        for statement in statements {
            match statement {
                SequentialStatement::DeviceCall(call) => {
                    sites.push(Site::Call(call, context.clone()))
                }
                SequentialStatement::Assignment(assignment) => {
                    sites.push(Site::Assign(assignment, context.clone()));
                }
                SequentialStatement::Parallel(parallel) => {
                    for call in &parallel.calls {
                        sites.push(Site::Call(call, context.clone()));
                    }
                }
                SequentialStatement::If(branching) => {
                    // Reaching any branch, `else` included, reveals every condition.
                    let mut inner = context.clone();
                    for branch in &branching.branches {
                        inner.raise(self.class_of_expr(&branch.condition.expr), || {
                            format!("condition `{}`", branch.condition.source)
                        });
                    }
                    for branch in &branching.branches {
                        self.sites(&branch.statements, &inner, sites);
                    }
                    self.sites(&branching.otherwise, &inner, sites);
                }
                SequentialStatement::Declassify(release) => {
                    sites.push(Site::Declassify(release, context.clone()));
                }
                SequentialStatement::Assert { .. } | SequentialStatement::Null { .. } => {}
            }
        }
    }

    /// A sensitivity name must be an event the runtime raises: `signal` /
    /// `signal.changed`, `timer`, `barrier` / `barrier.ready`, or
    /// `device.done` / `device.failed` / `device.timeout`. Anything else
    /// would never wake the process.
    fn sensitivity_error(&self, trigger: &str) -> Option<String> {
        let (root, member) = match trigger.split_once('.') {
            Some((root, member)) => (root, Some(member)),
            None => (trigger, None),
        };
        let (what, allowed): (&str, &[Option<&str>]) = if self.values.contains_key(root) {
            ("signal", &[None, Some("changed")])
        } else if self.timers.contains(root) {
            ("timer", &[None])
        } else if self.barriers.contains(root) {
            ("barrier", &[None, Some("ready")])
        } else if self.devices.contains_key(root) {
            ("device", &[Some("done"), Some("failed"), Some("timeout")])
        } else {
            return Some(format!("unknown process sensitivity name: {trigger}"));
        };
        if allowed.contains(&member) {
            return None;
        }
        let expected = allowed
            .iter()
            .map(|member| match member {
                Some(member) => format!("{root}.{member}"),
                None => root.to_owned(),
            })
            .collect::<Vec<_>>()
            .join(", ");
        Some(format!(
            "{trigger} never fires: {what} {root} raises only {expected}"
        ))
    }

    /// The `(from, to)` range of a valid declassifier.
    fn declassifier_range(&self, name: &str) -> Option<(Class, Class)> {
        let device = self.devices.get(name)?;
        if device.device_type != "declassifier" {
            return None;
        }
        let class = |key: &str| {
            device
                .generic(key)
                .and_then(|value| value.as_ident())
                .and_then(parse_class)
        };
        Some((class("from")?, class("to")?))
    }

    /// E219: a declassifier must name a range that lowers and at least one
    /// trusted means (a local deterministic filter, human approval, or both).
    fn check_declassifiers(&self, diagnostics: &mut Vec<Diagnostic>) {
        for device in self.devices.values() {
            if device.device_type != "declassifier" {
                continue;
            }
            let mut problems = Vec::new();
            let ident = |key: &str| device.generic(key).and_then(|value| value.as_ident());
            let class = |key: &str| ident(key).and_then(parse_class);
            match (class("from"), class("to")) {
                (Some(from), Some(to)) if to >= from => {
                    problems.push(format!("to {to:?} is not weaker than from {from:?}"));
                }
                (Some(_), Some(_)) => {}
                _ => problems.push("needs from => <class> and to => <class>".to_owned()),
            }
            let filter = device.generic("filter");
            let approval = device.generic("approval");
            if filter.is_none() && approval.is_none() {
                problems.push("needs filter => <device>, approval => human, or both".to_owned());
            }
            if let Some(filter) = filter {
                match filter.as_ident().and_then(|name| self.devices.get(name)) {
                    Some(checker)
                        if checker.device_type == "deterministic"
                            && location_of(checker) == Location::Local => {}
                    _ => problems.push("filter must name a local deterministic device".to_owned()),
                }
                if ident("filter_method").is_none() {
                    problems.push("filter needs filter_method => <method>".to_owned());
                }
            }
            if approval.is_some_and(|value| value.as_ident() != Some("human")) {
                problems.push("approval must be human".to_owned());
            }
            if location_of(device) != Location::Local {
                problems.push("a declassifier must be local".to_owned());
            }
            for problem in problems {
                diagnostics.push(diag(
                    "AWHDL-E219",
                    format!("declassifier {}: {problem}", device.name),
                    device.span,
                ));
            }
        }
    }

    fn check_declassify(&self, release: &awhdl_ast::Declassify, diagnostics: &mut Vec<Diagnostic>) {
        self.check_expression(&release.value, diagnostics);
        let target = match self.values.get(&release.target) {
            None => {
                diagnostics.push(diag(
                    "AWHDL-E209",
                    format!(
                        "declassify writes unknown signal or port: {}",
                        release.target
                    ),
                    release.span,
                ));
                None
            }
            Some(target) => {
                if target.input_port {
                    diagnostics.push(diag(
                        "AWHDL-E217",
                        format!("cannot write input port {}", release.target),
                        release.span,
                    ));
                }
                Some(target)
            }
        };
        let Some(device) = self.devices.get(&release.declassifier) else {
            diagnostics.push(diag(
                "AWHDL-E207",
                format!("declassify uses unknown device: {}", release.declassifier),
                release.span,
            ));
            return;
        };
        if device.device_type != "declassifier" {
            diagnostics.push(diag(
                "AWHDL-E220",
                format!("{} is not a declassifier", release.declassifier),
                release.span,
            ));
            return;
        }
        // An invalid declassifier is reported once, as E219.
        let Some((from, to)) = self.declassifier_range(&release.declassifier) else {
            return;
        };
        let source = self.class_of_expr(&release.value.expr);
        if source > from {
            diagnostics.push(diag(
                "AWHDL-E307",
                format!(
                    "{source:?} data exceeds the {from:?} range of declassifier {}",
                    release.declassifier
                ),
                release.span,
            ));
        }
        if let Some(target) = target
            && to > target.class
        {
            diagnostics.push(diag(
                "AWHDL-E303",
                format!(
                    "declassify to {to:?} cannot be stored in {:?} signal {}",
                    target.class, release.target
                ),
                release.span,
            ));
        }
    }

    fn check_sequential(&self, statement: &SequentialStatement, diagnostics: &mut Vec<Diagnostic>) {
        match statement {
            SequentialStatement::DeviceCall(call) => self.check_call(call, diagnostics),
            SequentialStatement::Assignment(assignment) => {
                self.check_expression(&assignment.value, diagnostics);
                match self.values.get(&assignment.target) {
                    None => diagnostics.push(diag(
                        "AWHDL-E209",
                        format!(
                            "assignment writes unknown signal or port: {}",
                            assignment.target
                        ),
                        assignment.span,
                    )),
                    Some(target) => {
                        if target.input_port {
                            diagnostics.push(diag(
                                "AWHDL-E217",
                                format!("cannot write input port {}", assignment.target),
                                assignment.span,
                            ));
                        }
                        let source = self.class_of_expr(&assignment.value.expr);
                        if source > target.class {
                            diagnostics.push(diag(
                                "AWHDL-E303",
                                format!(
                                    "assignment lowers {source:?} data into {:?} signal {}; only declassify ... using <declassifier> may lower a class",
                                    target.class, assignment.target
                                ),
                                assignment.span,
                            ));
                        }
                    }
                }
            }
            SequentialStatement::If(branching) => {
                for branch in &branching.branches {
                    self.check_expression(&branch.condition, diagnostics);
                    for nested in &branch.statements {
                        self.check_sequential(nested, diagnostics);
                    }
                }
                for nested in &branching.otherwise {
                    self.check_sequential(nested, diagnostics);
                }
            }
            SequentialStatement::Parallel(parallel) => {
                let mut outputs = BTreeSet::new();
                for call in &parallel.calls {
                    if !outputs.insert(&call.output) {
                        diagnostics.push(diag(
                            "AWHDL-E215",
                            format!("parallel branches both write {}", call.output),
                            call.span,
                        ));
                    }
                    self.check_call(call, diagnostics);
                }
            }
            SequentialStatement::Assert { condition, .. } => {
                self.check_expression(condition, diagnostics);
            }
            SequentialStatement::Declassify(release) => self.check_declassify(release, diagnostics),
            SequentialStatement::Null { .. } => {}
        }
    }

    fn check_call(&self, call: &DeviceCall, diagnostics: &mut Vec<Diagnostic>) {
        if self
            .devices
            .get(&call.device)
            .is_some_and(|device| device.device_type == "declassifier")
        {
            diagnostics.push(diag(
                "AWHDL-E220",
                format!(
                    "declassifier {} cannot be called; use declassify ... using {}",
                    call.device, call.device
                ),
                call.span,
            ));
        }
        for argument in &call.arguments {
            self.check_expression(argument, diagnostics);
        }
        if call.timeout.is_some_and(|time| time.millis == 0) {
            diagnostics.push(diag(
                "AWHDL-E211",
                "call timeout must be positive".to_owned(),
                call.span,
            ));
        }
        let flowing = call
            .arguments
            .iter()
            .map(|argument| self.class_of_expr(&argument.expr))
            .max()
            .unwrap_or(Class::Public);
        match self.devices.get(&call.device) {
            None => diagnostics.push(diag(
                "AWHDL-E207",
                format!("call references unknown device: {}", call.device),
                call.span,
            )),
            Some(device) => {
                if location_of(device) == Location::Cloud
                    && self.cloud_floor.is_some_and(|floor| flowing >= floor)
                {
                    diagnostics.push(diag(
                        "AWHDL-E301",
                        format!(
                            "{flowing:?} data cannot flow to cloud device {}",
                            call.device
                        ),
                        call.span,
                    ));
                }
                if let Some(clearance) = device
                    .generic("clearance")
                    .and_then(|value| value.as_ident())
                    .and_then(parse_class)
                    && flowing > clearance
                {
                    diagnostics.push(diag(
                        "AWHDL-E302",
                        format!(
                            "{flowing:?} data exceeds {clearance:?} clearance of device {}",
                            call.device
                        ),
                        call.span,
                    ));
                }
            }
        }
        match self.values.get(&call.output) {
            None => diagnostics.push(diag(
                "AWHDL-E208",
                format!("call writes unknown signal or port: {}", call.output),
                call.span,
            )),
            Some(output) => {
                if output.input_port {
                    diagnostics.push(diag(
                        "AWHDL-E217",
                        format!("cannot write input port {}", call.output),
                        call.span,
                    ));
                }
                // A device result is derived from its inputs and keeps their class.
                if flowing > output.class {
                    diagnostics.push(diag(
                        "AWHDL-E303",
                        format!(
                            "result of {}.{} derived from {flowing:?} data cannot be stored in {:?} signal {}",
                            call.device, call.method, output.class, call.output
                        ),
                        call.span,
                    ));
                }
            }
        }
    }

    fn check_expression(&self, expression: &Expression, diagnostics: &mut Vec<Diagnostic>) {
        for path in expression.expr.names() {
            let root = path[0].as_str();
            let known = self.values.contains_key(root)
                || self.devices.contains_key(root)
                || self.timers.contains(root)
                || self.barriers.contains(root);
            if !known {
                diagnostics.push(diag(
                    "AWHDL-E210",
                    format!("unknown name in expression: {}", path.join(".")),
                    expression.span,
                ));
            }
        }
    }

    /// Strongest class of any value the expression reads. Device, timer and
    /// barrier events carry no data; literals are public.
    fn class_of_expr(&self, expr: &Expr) -> Class {
        expr.names()
            .into_iter()
            .filter_map(|path| self.values.get(&path[0]).map(|value| value.class))
            .max()
            .unwrap_or(Class::Public)
    }
}

fn check_device(device: &DeviceDecl, diagnostics: &mut Vec<Diagnostic>) {
    let mut keys = BTreeSet::new();
    for generic in &device.generics {
        if !keys.insert(&generic.key) {
            diagnostics.push(diag(
                "AWHDL-E218",
                format!(
                    "duplicate generic {} on device {}",
                    generic.key, device.name
                ),
                generic.span,
            ));
        }
        let (supported, deferred, what): (fn(&str) -> bool, &[&str], &str) =
            match generic.key.as_str() {
                "location" => (
                    |name| parse_location(name).is_some(),
                    &DEFERRED_LOCATIONS,
                    "location",
                ),
                "clearance" => (
                    |name| parse_class(name).is_some(),
                    &DEFERRED_CLASSES,
                    "classification",
                ),
                _ => continue,
            };
        match generic.value.as_ident() {
            Some(name) => profile_name(
                name,
                supported(name),
                deferred,
                what,
                generic.span,
                diagnostics,
            ),
            None => diagnostics.push(diag(
                "AWHDL-E218",
                format!("{} of device {} must be a name", generic.key, device.name),
                generic.span,
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use awhdl_parser::parse;

    fn codes(source: &str) -> Vec<&'static str> {
        check(&parse(source).unwrap())
            .into_iter()
            .map(|item| item.code)
            .collect()
    }

    #[test]
    fn accepts_the_minimal_hello_structure() {
        let source = include_str!("../../../examples/hello.awhdl");
        assert!(check(&parse(source).unwrap()).is_empty());
    }

    #[test]
    fn rejects_an_unknown_device_call() {
        let source = r#"
entity hello is
    port (task : in text<internal>; result : out text<internal>);
end hello;
architecture behavioral of hello is
begin
    process(task)
    begin
        missing.run(task) -> result;
    end process;
end behavioral;
"#;
        let diagnostics = check(&parse(source).unwrap());
        assert!(diagnostics.iter().any(|item| item.code == "AWHDL-E207"));
    }

    const FLOW: &str = r#"
entity flow is
    port (
        patient : in  table<restricted>;
        notes   : in  text<internal>;
        ping    : in  text<public>;
        report  : out text<internal>
    );
end flow;
architecture secure of flow is
    device local_llm : agent generic (location => local, clearance => restricted);
    device reviewer : agent generic (location => cloud, clearance => internal);
    signal summary : text<internal>;
    signal private_summary : text<restricted>;
    signal count : number<public> := 0;
begin
    process(ping)
    begin
        BODY
    end process;
end secure;
"#;

    fn flow(body: &str) -> Vec<&'static str> {
        codes(&FLOW.replace("BODY", body))
    }

    fn triggered(trigger: &str, body: &str) -> Vec<&'static str> {
        codes(
            &FLOW
                .replace("process(ping)", &format!("process({trigger})"))
                .replace("BODY", body),
        )
    }

    #[test]
    fn restricted_data_cannot_reach_a_cloud_device() {
        assert!(flow("reviewer.review(notes) -> report;").is_empty());
        assert!(flow("local_llm.run(patient) -> private_summary;").is_empty());
        let direct = flow("reviewer.review(patient) -> report;");
        assert!(direct.contains(&"AWHDL-E301") && direct.contains(&"AWHDL-E302"));
        // Laundering through a local device into a lower label is rejected at the store.
        assert!(flow("local_llm.run(patient) -> summary;").contains(&"AWHDL-E303"));
        assert!(flow("summary <= patient;").contains(&"AWHDL-E303"));
        // Derived values inherit the strongest input class, even inside expressions.
        assert!(
            flow("reviewer.review(notes, count + 1, private_summary) -> report;")
                .contains(&"AWHDL-E301")
        );
        // Branches and parallel blocks are checked the same way.
        assert!(
            flow("if count > 1 then parallel reviewer.review(private_summary) -> report; end parallel; end if;")
                .contains(&"AWHDL-E301")
        );
    }

    #[test]
    fn implicit_flows_through_triggers_conditions_and_outcomes_are_rejected() {
        // A restricted trigger makes everything the process does restricted.
        assert_eq!(triggered("patient", "report <= \"seen\";"), ["AWHDL-E304"]);
        let reveal = triggered("patient", "reviewer.review(count) -> summary;");
        assert!(reveal.contains(&"AWHDL-E305") && reveal.contains(&"AWHDL-E306"));
        // A condition taints its branches, `else` included.
        assert_eq!(
            flow("if private_summary = \"x\" then report <= \"yes\"; end if;"),
            ["AWHDL-E304"]
        );
        assert_eq!(
            flow("if private_summary = \"x\" then null; else report <= \"no\"; end if;"),
            ["AWHDL-E304"]
        );
        assert!(
            flow("if private_summary = \"x\" then reviewer.review(count) -> summary; end if;")
                .contains(&"AWHDL-E305")
        );
        // A device's outcome events carry the class of what it was given.
        let events = FLOW
            .replace("BODY", "local_llm.run(patient) -> private_summary;")
            .replace(
                "end secure;",
                "process(local_llm.done)\nbegin\n    report <= \"done\";\nend process;\nend secure;",
            );
        assert_eq!(codes(&events), ["AWHDL-E304"]);
        // `on timeout` reveals how long the body's calls took.
        let timed = FLOW
            .replace("process(ping)", "process(ping) timeout 1 min;")
            .replace(
                "BODY",
                "local_llm.run(patient) -> private_summary;\n    on timeout\n        report <= \"late\";",
            );
        assert_eq!(codes(&timed), ["AWHDL-E304"]);
        // An explicit violation is reported once, not again as an implicit one.
        assert_eq!(triggered("patient", "summary <= patient;"), ["AWHDL-E303"]);
        // Public triggers and public conditions stay allowed.
        assert!(flow("if count > 1 then reviewer.review(notes) -> report; end if;").is_empty());
        assert!(triggered("notes", "reviewer.review(notes) -> report;").is_empty());
    }

    const RELEASE: &str =
        "device pii_check : deterministic generic (location => local, route => \"pii\");
    device release : declassifier generic (from => restricted, to => internal,
        filter => pii_check, filter_method => scan, approval => human);
    device reviewer";

    fn released(body: &str) -> Vec<&'static str> {
        codes(
            &FLOW
                .replacen("device reviewer", RELEASE, 1)
                .replace("BODY", body),
        )
    }

    #[test]
    fn only_a_declassifier_lowers_a_class() {
        // Released data may go where its new class may go.
        assert!(released("summary <= declassify private_summary using release;").is_empty());
        assert!(
            released(
                "summary <= declassify private_summary using release;\n        reviewer.review(summary) -> report;"
            )
            .is_empty()
        );
        // Without a release the same store is laundering (E303).
        assert_eq!(released("summary <= private_summary;"), ["AWHDL-E303"]);
        // A declassifier is not a device to call, and only declassifiers release.
        assert!(released("release.run(private_summary) -> summary;").contains(&"AWHDL-E220"));
        assert_eq!(
            released("summary <= declassify private_summary using local_llm;"),
            ["AWHDL-E220"]
        );
        // The release keeps its `to` class and the context stays checked.
        assert_eq!(
            released("count <= declassify private_summary using release;"),
            ["AWHDL-E303"]
        );
        // A human approver sees the release in its context, up to `from`;
        // a filter-only declassifier sees only the content.
        let in_context = |release: &str| {
            codes(
                &FLOW
                    .replacen("device reviewer", release, 1)
                    .replace("process(ping)", "process(patient)")
                    .replace(
                        "BODY",
                        "summary <= declassify private_summary using release;",
                    ),
            )
        };
        assert!(in_context(RELEASE).is_empty());
        let filter_only = RELEASE.replace(", approval => human", "");
        assert_eq!(in_context(&filter_only), ["AWHDL-E304"]);
        // The release's outcome events carry the released class.
        let events = FLOW
            .replacen("device reviewer", RELEASE, 1)
            .replace("BODY", "summary <= declassify private_summary using release;")
            .replace(
                "end secure;",
                "process(release.failed)\nbegin\n    report <= \"refused\";\nend process;\nend secure;",
            );
        assert_eq!(codes(&events), ["AWHDL-E304"]);
    }

    #[test]
    fn declassifiers_must_lower_through_trusted_means() {
        let with = |generics: &str| {
            codes(
                &FLOW
                    .replacen(
                        "device reviewer",
                        &format!(
                            "device pii_check : deterministic generic (location => local, route => \"pii\");\n    device release : declassifier generic ({generics});\n    device reviewer"
                        ),
                        1,
                    )
                    .replace("BODY", "null;"),
            )
        };
        // Each means alone, and both together, are valid.
        assert!(with("from => restricted, to => internal, approval => human").is_empty());
        assert!(
            with("from => restricted, to => public, filter => pii_check, filter_method => scan")
                .is_empty()
        );
        for invalid in [
            "from => restricted, to => internal",
            "from => internal, to => restricted, approval => human",
            "to => internal, approval => human",
            "from => restricted, to => internal, approval => operator",
            "from => restricted, to => internal, filter => pii_check",
            "from => restricted, to => internal, filter => local_llm, filter_method => scan",
            "from => restricted, to => internal, approval => human, location => cloud",
        ] {
            assert!(with(invalid).contains(&"AWHDL-E219"), "{invalid}");
        }
        // Data above the declassifier's range cannot be released.
        let narrow = FLOW
            .replacen(
                "device reviewer",
                "device release : declassifier generic (from => internal, to => public, approval => human);\n    device reviewer",
                1,
            )
            .replace("BODY", "count <= declassify private_summary using release;");
        assert!(codes(&narrow).contains(&"AWHDL-E307"));
    }

    #[test]
    fn declared_never_flow_lowers_the_cloud_floor() {
        let strict = FLOW.replace(
            "begin\n    process",
            "begin\n    assert never (internal -> cloud);\n    process",
        );
        assert!(
            codes(&strict.replace("BODY", "reviewer.review(notes) -> report;"))
                .contains(&"AWHDL-E301")
        );
        assert!(codes(&strict.replace("BODY", "reviewer.review(count) -> report;")).is_empty());
    }

    #[test]
    fn unlabeled_data_is_treated_as_restricted() {
        let source = FLOW.replace("notes   : in  text<internal>;", "notes   : in  text;");
        assert!(
            codes(&source.replace("BODY", "reviewer.review(notes) -> report;"))
                .contains(&"AWHDL-E301")
        );
    }

    #[test]
    fn features_outside_the_v01_profile_are_reported() {
        let secret = FLOW.replace("text<restricted>", "text<secret>");
        assert!(codes(&secret.replace("BODY", "null;")).contains(&"AWHDL-E401"));
        let bogus = FLOW.replace("text<restricted>", "text<topsecret>");
        assert!(codes(&bogus.replace("BODY", "null;")).contains(&"AWHDL-E402"));
        let sandbox = FLOW.replace("location => local", "location => sandbox");
        assert!(codes(&sandbox.replace("BODY", "null;")).contains(&"AWHDL-E401"));
        let declassifier = FLOW.replace(
            "device reviewer : agent",
            "device anonymizer : declassifier;\n    device reviewer : agent",
        );
        assert!(codes(&declassifier.replace("BODY", "null;")).contains(&"AWHDL-E219"));
    }

    #[test]
    fn sensitivity_names_must_be_events_the_runtime_raises() {
        let process = |names: &str| {
            let source = FLOW
                .replace(
                    "signal count",
                    "timer tick : period 1 sec;\n    barrier both (summary, private_summary);\n    signal count",
                )
                .replace("process(ping)", &format!("process({names})"));
            codes(&source.replace("BODY", "null;"))
        };
        for fires in [
            "patient",
            "notes.changed",
            "tick",
            "both",
            "both.ready",
            "local_llm.done",
            "local_llm.failed",
            "local_llm.timeout",
        ] {
            assert!(process(fires).is_empty(), "{fires} was rejected");
        }
        for never in [
            "local_llm",
            "local_llm.completed",
            "notes.field",
            "tick.ready",
            "both.done",
            "missing",
        ] {
            assert_eq!(process(never), ["AWHDL-E206"], "{never} was accepted");
        }
    }

    #[test]
    fn structural_rules_for_v01_constructs() {
        let body = |body: &str| flow(body);
        assert!(body("patient <= summary;").contains(&"AWHDL-E217"));
        assert!(body("report <= missing + 1;").contains(&"AWHDL-E210"));
        assert!(
            body("parallel local_llm.a(notes) -> private_summary; local_llm.b(notes) -> private_summary; end parallel;")
                .contains(&"AWHDL-E215")
        );
        assert!(
            body("local_llm.run(notes) timeout 0 sec -> private_summary;").contains(&"AWHDL-E211")
        );
        let declarations = FLOW
            .replace(
                "signal count",
                "timer tick : period 0 sec;\n    budget b is iterations <= 0; wall_time <= 5; end budget;\n    barrier ready_all (summary, nothing);\n    signal count",
            )
            .replace("process(ping)", "process(tick, ready_all.ready, ready_all.bogus)");
        let found = codes(&declarations.replace("BODY", "null;"));
        for code in ["AWHDL-E211", "AWHDL-E212", "AWHDL-E214", "AWHDL-E206"] {
            assert!(found.contains(&code), "{code} in {found:?}");
        }
    }
}
