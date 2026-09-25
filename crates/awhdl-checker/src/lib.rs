//! Static checks. Structural errors are `AWHDL-E2xx`, information-flow errors
//! are `AWHDL-E3xx`, and constructs outside the v0.1 profile are `AWHDL-E4xx`.
use awhdl_ast::{
    ArchitectureDecl, Assertion, BudgetValue, ConcurrentStatement, DataType, Declaration, Design,
    DesignUnit, DeviceCall, DeviceDecl, Expr, Expression, PortMode, SequentialStatement, Span,
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
const DEFERRED_DEVICE_KINDS: [&str; 1] = ["declassifier"];
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
                                    "assignment lowers {source:?} data into {:?} signal {}; declassification is not in the v0.1 profile",
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
            SequentialStatement::Null { .. } => {}
        }
    }

    fn check_call(&self, call: &DeviceCall, diagnostics: &mut Vec<Diagnostic>) {
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
    if DEFERRED_DEVICE_KINDS.contains(&device.device_type.as_str()) {
        diagnostics.push(diag(
            "AWHDL-E401",
            format!(
                "device kind {} is not in the v0.1 profile",
                device.device_type
            ),
            device.span,
        ));
    }
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
    process(patient, notes)
    begin
        BODY
    end process;
end secure;
"#;

    fn flow(body: &str) -> Vec<&'static str> {
        codes(&FLOW.replace("BODY", body))
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
        assert!(codes(&declassifier.replace("BODY", "null;")).contains(&"AWHDL-E401"));
    }

    #[test]
    fn sensitivity_names_must_be_events_the_runtime_raises() {
        let process = |names: &str| {
            let source = FLOW
                .replace(
                    "signal count",
                    "timer tick : period 1 sec;\n    barrier both (summary, private_summary);\n    signal count",
                )
                .replace("process(patient, notes)", &format!("process({names})"));
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
            .replace("process(patient, notes)", "process(tick, ready_all.ready, ready_all.bogus)");
        let found = codes(&declarations.replace("BODY", "null;"));
        for code in ["AWHDL-E211", "AWHDL-E212", "AWHDL-E214", "AWHDL-E206"] {
            assert!(found.contains(&code), "{code} in {found:?}");
        }
    }
}
