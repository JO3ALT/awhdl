//! View projectors: checked AST -> visualization IR.
use crate::{
    Edge, EdgeKind, GRAPH_VERSION, Graph, Group, Node, NodeKind, Options, ProjectionError,
    SOURCE_IR_VERSION, SourceRef, View,
};
use awhdl_ast::{
    ArchitectureDecl, Assertion, BarrierDecl, BudgetValue, ConcurrentStatement, DataType, Design,
    DesignUnit, DeviceCall, DeviceDecl, EntityDecl, GenericValue, PortMode, ProcessStmt,
    SequentialStatement, Span, TimerDecl,
};
use awhdl_checker::{Class, Location, class_of, location_of, parse_class};
use std::collections::{BTreeMap, BTreeSet};

mod activity;
mod petri;

const JOIN_KEY: &str = "(run_id, correlation_id, generation)";

pub(crate) fn project(
    design: &Design,
    source: &str,
    view: View,
    options: &Options,
) -> Result<Graph, ProjectionError> {
    let symbols = Symbols::new(design, options.architecture.as_deref())?;
    let mut builder = Builder {
        source,
        options,
        view,
        nodes: Vec::new(),
        node_index: BTreeMap::new(),
        edges: Vec::new(),
        edge_index: BTreeMap::new(),
        groups: Vec::new(),
        annotations: vec![
            "Derived view of the checked design; it does not define execution semantics."
                .to_owned(),
        ],
    };
    match view {
        View::Structure => structure(&mut builder, &symbols),
        View::Behavior => behavior(&mut builder, &symbols),
        View::Petri => petri::petri(&mut builder, &symbols),
        View::Activity => activity::activity(&mut builder, &symbols),
        View::Security => security(&mut builder, &symbols),
        View::State => {
            return Err(ProjectionError(format!(
                "architecture {} declares no workflow states: the v0.1 profile has no state \
                 types, and the state view does not infer them; use --view behavior or --view petri",
                symbols.architecture.name
            )));
        }
    }
    // A P/T net has only places and transitions, and an activity only its
    // own nodes; there, sequential asserts are actions (activity view).
    if options.show_policies && !matches!(view, View::Petri | View::Activity) {
        policies(&mut builder, &symbols);
    }
    if view == View::Behavior {
        builder.mark_loops();
    }
    Ok(builder.finish(&symbols))
}

// ---------------------------------------------------------------------------
// Symbols

struct ValueInfo<'a> {
    data_type: &'a DataType,
    port: Option<PortMode>,
    span: Span,
}

struct Symbols<'a> {
    entity: &'a EntityDecl,
    architecture: &'a ArchitectureDecl,
    /// Ports then signals, in declaration order.
    value_order: Vec<&'a str>,
    values: BTreeMap<&'a str, ValueInfo<'a>>,
    devices: Vec<&'a DeviceDecl>,
    timers: Vec<&'a TimerDecl>,
    barriers: Vec<&'a BarrierDecl>,
    processes: Vec<&'a ProcessStmt>,
    /// Every sensitivity name used by some process, e.g. `coder.done`.
    triggers: BTreeSet<String>,
}

impl<'a> Symbols<'a> {
    fn new(design: &'a Design, wanted: Option<&str>) -> Result<Self, ProjectionError> {
        let architectures = design
            .units
            .iter()
            .filter_map(|unit| match unit {
                DesignUnit::Architecture(architecture) => Some(architecture),
                DesignUnit::Entity(_) => None,
            })
            .collect::<Vec<_>>();
        let names = || {
            architectures
                .iter()
                .map(|architecture| architecture.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let architecture = match wanted {
            Some(name) => architectures
                .iter()
                .find(|architecture| architecture.name == name)
                .ok_or_else(|| {
                    ProjectionError(format!("no architecture named {name} (found: {})", names()))
                })?,
            None => match architectures.as_slice() {
                [only] => only,
                [] => return Err(ProjectionError("the file has no architecture".to_owned())),
                _ => {
                    return Err(ProjectionError(format!(
                        "several architectures ({}); choose one with --architecture",
                        names()
                    )));
                }
            },
        };
        let entity = design
            .units
            .iter()
            .find_map(|unit| match unit {
                DesignUnit::Entity(entity) if entity.name == architecture.entity => Some(entity),
                _ => None,
            })
            .ok_or_else(|| ProjectionError(format!("unknown entity {}", architecture.entity)))?;

        let mut symbols = Symbols {
            entity,
            architecture,
            value_order: Vec::new(),
            values: BTreeMap::new(),
            devices: Vec::new(),
            timers: Vec::new(),
            barriers: Vec::new(),
            processes: Vec::new(),
            triggers: BTreeSet::new(),
        };
        for port in &entity.ports {
            for name in &port.names {
                symbols.value_order.push(name);
                symbols.values.insert(
                    name,
                    ValueInfo {
                        data_type: &port.data_type,
                        port: Some(port.mode),
                        span: port.span,
                    },
                );
            }
        }
        for declaration in &architecture.declarations {
            match declaration {
                awhdl_ast::Declaration::Device(device) => symbols.devices.push(device),
                awhdl_ast::Declaration::Signal(signal) => {
                    for name in &signal.names {
                        symbols.value_order.push(name);
                        symbols.values.insert(
                            name,
                            ValueInfo {
                                data_type: &signal.data_type,
                                port: None,
                                span: signal.span,
                            },
                        );
                    }
                }
                awhdl_ast::Declaration::Timer(timer) => symbols.timers.push(timer),
                awhdl_ast::Declaration::Barrier(barrier) => symbols.barriers.push(barrier),
                awhdl_ast::Declaration::Budget(_) => {}
            }
        }
        for statement in &architecture.statements {
            if let ConcurrentStatement::Process(process) = statement {
                symbols.processes.push(process);
                symbols.triggers.extend(process.sensitivity.iter().cloned());
            }
        }
        Ok(symbols)
    }

    fn device(&self, name: &str) -> Option<&'a DeviceDecl> {
        self.devices
            .iter()
            .copied()
            .find(|device| device.name == name)
    }

    fn class(&self, name: &str) -> Option<Class> {
        self.values.get(name).map(|info| class_of(info.data_type))
    }

    /// Value names read by an expression-like list of name paths.
    fn reads<'e>(&self, paths: Vec<&'e [String]>) -> Vec<&'e str> {
        let mut seen = Vec::new();
        for path in paths {
            let root = path[0].as_str();
            if self.values.contains_key(root) && !seen.contains(&root) {
                seen.push(root);
            }
        }
        seen
    }

    fn call_reads<'e>(&self, call: &'e DeviceCall) -> Vec<&'e str> {
        self.reads(
            call.arguments
                .iter()
                .flat_map(|argument| argument.expr.names())
                .collect(),
        )
    }

    /// Whether `device.outcome` wakes a process.
    fn subscribed(&self, device: &str, outcome: &str) -> bool {
        self.triggers.contains(&format!("{device}.{outcome}"))
    }

    /// The minimum class that may never reach a cloud device, with its sources.
    fn cloud_floor(&self) -> (Class, Vec<String>) {
        let mut floor = Class::Restricted;
        let mut sources = vec!["built-in: restricted -> cloud".to_owned()];
        for statement in &self.architecture.statements {
            if let ConcurrentStatement::Assert(assert) = statement
                && let Assertion::NeverFlow {
                    classification,
                    location,
                } = &assert.assertion
                && location == "cloud"
                && let Some(class) = parse_class(classification)
            {
                floor = floor.min(class);
                sources.push(format!("assert never ({classification} -> cloud)"));
            }
        }
        (floor, sources)
    }
}

enum Trigger<'s> {
    Value(&'s str),
    Timer(&'s TimerDecl),
    Barrier(&'s BarrierDecl),
    /// `device.done`, `device.failed`, `device.timeout`, or another event name.
    Event(&'s str),
}

fn trigger<'s>(symbols: &Symbols<'s>, text: &'s str) -> Trigger<'s> {
    let root = text.split('.').next().unwrap_or(text);
    if symbols.values.contains_key(root) {
        return Trigger::Value(root);
    }
    if let Some(timer) = symbols.timers.iter().find(|timer| timer.name == root) {
        return Trigger::Timer(timer);
    }
    if let Some(barrier) = symbols.barriers.iter().find(|barrier| barrier.name == root) {
        return Trigger::Barrier(barrier);
    }
    Trigger::Event(text)
}

/// Visit every statement, nested ones included, in source order.
fn each_statement<'s>(
    statements: &'s [SequentialStatement],
    visit: &mut dyn FnMut(&'s SequentialStatement),
) {
    for statement in statements {
        visit(statement);
        match statement {
            SequentialStatement::If(branching) => {
                for branch in &branching.branches {
                    each_statement(&branch.statements, visit);
                }
                each_statement(&branching.otherwise, visit);
            }
            SequentialStatement::Parallel(_)
            | SequentialStatement::DeviceCall(_)
            | SequentialStatement::Assignment(_)
            | SequentialStatement::Assert { .. }
            | SequentialStatement::Null { .. } => {}
        }
    }
}

fn process_statements(process: &ProcessStmt) -> Vec<&SequentialStatement> {
    let mut all = Vec::new();
    each_statement(&process.statements, &mut |statement| all.push(statement));
    each_statement(&process.on_timeout, &mut |statement| all.push(statement));
    all
}

fn calls_of(statement: &SequentialStatement) -> Vec<&DeviceCall> {
    match statement {
        SequentialStatement::DeviceCall(call) => vec![call],
        SequentialStatement::Parallel(parallel) => parallel.calls.iter().collect(),
        _ => Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Builder

struct Builder<'a> {
    source: &'a str,
    options: &'a Options,
    view: View,
    nodes: Vec<Node>,
    node_index: BTreeMap<String, usize>,
    edges: Vec<Edge>,
    edge_index: BTreeMap<String, usize>,
    groups: Vec<Group>,
    annotations: Vec<String>,
}

impl Builder<'_> {
    fn detail(&self) -> bool {
        !self.options.compact
    }

    fn source_ref(&self, span: Span) -> SourceRef {
        let before = &self.source[..span.start.min(self.source.len())];
        SourceRef {
            line: before.bytes().filter(|byte| *byte == b'\n').count() + 1,
            column: before
                .rsplit_once('\n')
                .map_or(before.len(), |(_, tail)| tail.len())
                + 1,
            start: span.start,
            end: span.end,
        }
    }

    fn has_node(&self, id: &str) -> bool {
        self.node_index.contains_key(id)
    }

    /// Get or create a node. `extra` lines are dropped by `--compact`;
    /// `significant` lines (classification, location, timeout) are kept.
    fn node(
        &mut self,
        id: &str,
        kind: NodeKind,
        name: &str,
        span: Option<Span>,
        extra: Vec<String>,
        significant: Vec<String>,
    ) -> &mut Node {
        if let Some(&index) = self.node_index.get(id) {
            return &mut self.nodes[index];
        }
        let label = if self.options.compact {
            vec![
                std::iter::once(name.to_owned())
                    .chain(significant)
                    .collect::<Vec<_>>()
                    .join(" · "),
            ]
        } else {
            std::iter::once(name.to_owned())
                .chain(extra)
                .chain(significant)
                .collect()
        };
        let source_ref = span.map(|span| self.source_ref(span));
        self.node_index.insert(id.to_owned(), self.nodes.len());
        self.nodes.push(Node {
            id: id.to_owned(),
            kind,
            label,
            source_ref,
            classification: None,
            location: None,
            device_kind: None,
            capabilities: BTreeMap::new(),
            generation_sensitive: false,
            metadata: BTreeMap::new(),
            group: None,
        });
        self.nodes.last_mut().expect("just pushed")
    }

    /// Get or create the edge `kind:from->to`; a repeated edge merges labels.
    fn edge(&mut self, from: &str, to: &str, kind: EdgeKind, label: Option<String>) -> &mut Edge {
        let id = format!("{}:{from}->{to}", kind_name(kind));
        if let Some(&index) = self.edge_index.get(&id) {
            let edge = &mut self.edges[index];
            if let Some(label) = label {
                match &mut edge.label {
                    Some(existing) if existing.split("; ").any(|part| part == label) => {}
                    Some(existing) => {
                        existing.push_str("; ");
                        existing.push_str(&label);
                    }
                    None => edge.label = Some(label),
                }
            }
            return edge;
        }
        self.edge_index.insert(id.clone(), self.edges.len());
        self.edges.push(Edge {
            id,
            from: from.to_owned(),
            to: to.to_owned(),
            kind,
            label,
            classification: None,
            condition: None,
            asynchronous: false,
            guarded: false,
            denied: false,
            generation_binding: None,
            security_significant: false,
            metadata: BTreeMap::new(),
        });
        self.edges.last_mut().expect("just pushed")
    }

    fn group(&mut self, id: &str, kind: &str, label: &str, parent: Option<&str>) {
        if self.groups.iter().any(|group| group.id == id) {
            return;
        }
        self.groups.push(Group {
            id: id.to_owned(),
            kind: kind.to_owned(),
            label: label.to_owned(),
            parent: parent.map(str::to_owned),
        });
    }

    fn annotate(&mut self, text: &str) {
        if !self.annotations.iter().any(|existing| existing == text) {
            self.annotations.push(text.to_owned());
        }
    }

    fn show_class(&self) -> bool {
        self.options.show_classification || self.view == View::Security
    }

    fn value(&mut self, symbols: &Symbols, name: &str) -> String {
        let id = format!("value:{name}");
        if self.has_node(&id) {
            return id;
        }
        let Some(info) = symbols.values.get(name) else {
            self.node(&id, NodeKind::Value, name, None, Vec::new(), Vec::new());
            return id;
        };
        let class = class_name(class_of(info.data_type));
        let mut extra = vec![match info.port {
            Some(mode) => format!("{} port · {}", mode_name(mode), info.data_type.name),
            None => format!("signal · {}", info.data_type.name),
        }];
        let mut significant = Vec::new();
        let class_text = match info.data_type.classification {
            Some(_) => class.to_owned(),
            None => format!("{class} (unlabeled)"),
        };
        if self.show_class() {
            significant.push(class_text);
        } else if self.options.compact {
            extra.clear();
        }
        let port = info.port;
        let node = self.node(
            &id,
            NodeKind::Value,
            name,
            Some(info.span),
            extra,
            significant,
        );
        node.classification = Some(class.to_owned());
        if let Some(mode) = port {
            node.metadata
                .insert("port".to_owned(), mode_name(mode).to_owned());
        }
        id
    }

    fn device(&mut self, device: &DeviceDecl, group_parent: Option<&str>) -> String {
        let id = format!("device:{}", device.name);
        if self.has_node(&id) {
            return id;
        }
        let location = location_name(location_of(device));
        let clearance = device
            .generic("clearance")
            .and_then(GenericValue::as_ident)
            .map(str::to_owned);
        let capabilities = device
            .generics
            .iter()
            .filter(|generic| generic.key != "location" && generic.key != "clearance")
            .map(|generic| (generic.key.clone(), generic_text(&generic.value)))
            .collect::<BTreeMap<_, _>>();
        let mut extra = Vec::new();
        if self.options.show_capabilities {
            extra.extend(
                capabilities
                    .iter()
                    .map(|(key, value)| format!("{key} = {value}")),
            );
        }
        let mut significant = vec![format!("{} · {location}", device.device_type)];
        if let Some(clearance) = &clearance
            && self.show_class()
        {
            significant.push(format!("clearance {clearance}"));
        }
        let kind = match device.device_type.as_str() {
            "human" => NodeKind::Approval,
            "declassifier" => NodeKind::Declassifier,
            _ => NodeKind::Device,
        };
        let zone = format!("zone:{location}");
        self.group(&zone, "zone", &format!("{location} zone"), group_parent);
        let span = device.span;
        let node = self.node(&id, kind, &device.name, Some(span), extra, significant);
        node.location = Some(location.to_owned());
        node.device_kind = Some(device.device_type.clone());
        node.capabilities = capabilities;
        node.group = Some(zone);
        if let Some(clearance) = clearance {
            node.metadata.insert("clearance".to_owned(), clearance);
        }
        id
    }

    fn timer(&mut self, timer: &TimerDecl) -> String {
        let id = format!("timer:{}", timer.name);
        self.node(
            &id,
            NodeKind::Timer,
            &timer.name,
            Some(timer.span),
            Vec::new(),
            vec![format!("every {}", duration(timer.period.millis))],
        );
        id
    }

    fn barrier(&mut self, barrier: &BarrierDecl) -> String {
        let id = format!("barrier:{}", barrier.name);
        let node = self.node(
            &id,
            NodeKind::Barrier,
            &barrier.name,
            Some(barrier.span),
            vec![format!("barrier ({})", barrier.members.join(", "))],
            Vec::new(),
        );
        node.generation_sensitive = true;
        id
    }

    /// Mark the edges that close a cycle (a feedback or retry loop) as `retry`,
    /// found by a depth-first walk from the nodes nothing points at.
    fn mark_loops(&mut self) {
        let mut outgoing: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        let mut targets = BTreeSet::new();
        for (index, edge) in self.edges.iter().enumerate() {
            outgoing.entry(edge.from.as_str()).or_default().push(index);
            targets.insert(edge.to.as_str());
        }
        let roots = self
            .nodes
            .iter()
            .filter(|node| !targets.contains(node.id.as_str()))
            .chain(self.nodes.iter())
            .map(|node| node.id.as_str())
            .collect::<Vec<_>>();
        // 0 unseen, 1 on the current path, 2 finished.
        let mut state: BTreeMap<&str, u8> = BTreeMap::new();
        let mut back = BTreeSet::new();
        for root in roots {
            if state.contains_key(root) {
                continue;
            }
            let mut stack = vec![(root, 0usize)];
            state.insert(root, 1);
            while let Some((node, next)) = stack.pop() {
                let edges = outgoing.get(node).map(Vec::as_slice).unwrap_or_default();
                if let Some(&index) = edges.get(next) {
                    stack.push((node, next + 1));
                    let to = self.edges[index].to.as_str();
                    match state.get(to) {
                        Some(1) => {
                            back.insert(index);
                        }
                        Some(_) => {}
                        None => {
                            state.insert(to, 1);
                            stack.push((to, 0));
                        }
                    }
                } else {
                    state.insert(node, 2);
                }
            }
        }
        for index in back {
            let edge = &mut self.edges[index];
            edge.kind = EdgeKind::Retry;
            edge.id = format!("{}:{}->{}", kind_name(EdgeKind::Retry), edge.from, edge.to);
            edge.metadata
                .insert("loop".to_owned(), "closes a feedback cycle".to_owned());
            edge.label = Some(match edge.label.take() {
                Some(label) => format!("{label} · loop"),
                None => "loop".to_owned(),
            });
        }
    }

    fn finish(mut self, symbols: &Symbols) -> Graph {
        if self.view == View::Security {
            // Every flow of non-public data is what the security view is for.
            for edge in &mut self.edges {
                if edge
                    .classification
                    .as_deref()
                    .is_some_and(|class| class != "public")
                {
                    edge.security_significant = true;
                }
            }
        }
        if self.options.compact {
            for edge in &mut self.edges {
                if !edge.security_significant && edge.kind != EdgeKind::Retry {
                    edge.label = edge.condition.clone();
                }
            }
        }
        if self
            .edges
            .iter()
            .any(|edge| edge.generation_binding.is_some())
        {
            self.annotate(&format!(
                "Joins are generation-aware: results join only on the same {JOIN_KEY}."
            ));
        }
        Graph {
            graph_version: GRAPH_VERSION,
            source_ir_version: SOURCE_IR_VERSION,
            view: self.view,
            entity: symbols.entity.name.clone(),
            architecture: symbols.architecture.name.clone(),
            nodes: self.nodes,
            edges: self.edges,
            groups: self.groups,
            annotations: self.annotations,
        }
    }
}

// ---------------------------------------------------------------------------
// Structure view

fn structure(b: &mut Builder, s: &Symbols) {
    let entity_group = format!("entity:{}", s.entity.name);
    b.group(
        &entity_group,
        "entity",
        &format!(
            "entity {} · architecture {}",
            s.entity.name, s.architecture.name
        ),
        None,
    );
    for name in &s.value_order {
        let id = b.value(s, name);
        if s.values[name].port.is_none() {
            b.nodes[b.node_index[&id]].group = Some(entity_group.clone());
        }
    }
    for device in &s.devices {
        b.device(device, Some(&entity_group));
    }
    for timer in &s.timers {
        let id = b.timer(timer);
        b.nodes[b.node_index[&id]].group = Some(entity_group.clone());
    }
    for barrier in &s.barriers {
        let id = b.barrier(barrier);
        b.nodes[b.node_index[&id]].group = Some(entity_group.clone());
        for member in &barrier.members {
            let from = b.value(s, member);
            b.edge(&from, &id, EdgeKind::Join, None).generation_binding = Some(JOIN_KEY.to_owned());
        }
    }

    for process in &s.processes {
        let mut writes = Vec::new();
        for statement in process_statements(process) {
            for call in calls_of(statement) {
                let Some(device) = s.device(&call.device) else {
                    continue;
                };
                let device_id = b.device(device, Some(&entity_group));
                let cloud = location_of(device) == Location::Cloud;
                for read in s.call_reads(call) {
                    let from = b.value(s, read);
                    let class = s.class(read).map(class_name);
                    let label = b.show_class().then(|| class.unwrap_or("").to_owned());
                    let edge = b.edge(&from, &device_id, EdgeKind::DataFlow, label);
                    edge.classification = class.map(str::to_owned);
                    if cloud {
                        mark_cloud_egress(edge);
                    }
                }
                let to = b.value(s, &call.output);
                let detail = b.detail().then(|| call.method.clone());
                b.edge(&device_id, &to, EdgeKind::DataFlow, detail);
                writes.push(device_id);
            }
            if let SequentialStatement::Assignment(assignment) = statement {
                let to = b.value(s, &assignment.target);
                for read in s.reads(assignment.value.expr.names()) {
                    let from = b.value(s, read);
                    let detail = b.detail().then(|| "<=".to_owned());
                    b.edge(&from, &to, EdgeKind::DataFlow, detail);
                }
                writes.push(to);
            }
        }
        // What wakes the process reaches everything it drives.
        for text in &process.sensitivity {
            let from = match trigger(s, text) {
                Trigger::Value(name) => b.value(s, name),
                Trigger::Timer(timer) => b.timer(timer),
                Trigger::Barrier(barrier) => b.barrier(barrier),
                Trigger::Event(event) => match s.device(event.split('.').next().unwrap_or(event)) {
                    Some(device) => b.device(device, Some(&entity_group)),
                    None => continue,
                },
            };
            let label = text.split_once('.').map(|(_, member)| member.to_owned());
            for to in &writes {
                if *to != from {
                    let edge = b.edge(&from, to, EdgeKind::EventTrigger, label.clone());
                    edge.asynchronous = true;
                }
            }
        }
    }
    // A trigger edge beside a data-flow edge on the same pair adds nothing.
    let flows = b
        .edges
        .iter()
        .filter(|edge| edge.kind == EdgeKind::DataFlow)
        .map(|edge| (edge.from.clone(), edge.to.clone()))
        .collect::<BTreeSet<_>>();
    b.edges.retain(|edge| {
        edge.kind != EdgeKind::EventTrigger
            || !flows.contains(&(edge.from.clone(), edge.to.clone()))
    });
    b.edge_index = b
        .edges
        .iter()
        .enumerate()
        .map(|(index, edge)| (edge.id.clone(), index))
        .collect();
}

fn mark_cloud_egress(edge: &mut Edge) {
    edge.guarded = true;
    edge.security_significant = true;
    edge.metadata
        .insert("crossing".to_owned(), "local -> cloud".to_owned());
    let note = "cloud egress (checked at runtime)";
    match &mut edge.label {
        Some(label) if label.contains(note) => {}
        Some(label) if !label.is_empty() => {
            label.push_str(" · ");
            label.push_str(note);
        }
        _ => edge.label = Some(note.to_owned()),
    }
}

// ---------------------------------------------------------------------------
// Behavior view

#[derive(Default)]
struct Counters {
    calls: usize,
    decisions: usize,
    forks: usize,
    asserts: usize,
}

fn behavior(b: &mut Builder, s: &Symbols) {
    for (index, process) in s.processes.iter().enumerate() {
        let number = index + 1;
        let id = format!("process:{number}");
        let significant = process
            .timeout
            .map(|timeout| vec![format!("timeout {}", duration(timeout.millis))])
            .unwrap_or_default();
        b.node(
            &id,
            NodeKind::Process,
            &format!("process({})", process.sensitivity.join(", ")),
            Some(process.span),
            Vec::new(),
            significant,
        );
        for text in &process.sensitivity {
            let (from, label) = match trigger(s, text) {
                Trigger::Value(name) => (b.value(s, name), None),
                Trigger::Timer(timer) => (b.timer(timer), None),
                Trigger::Barrier(barrier) => (b.barrier(barrier), Some("ready".to_owned())),
                Trigger::Event(event) => (event_node(b, event), None),
            };
            b.edge(&from, &id, EdgeKind::EventTrigger, label)
                .asynchronous = true;
        }
        let mut counters = Counters::default();
        walk_behavior(b, s, number, &mut counters, &process.statements, &id, None);
        if let Some(timeout) = process.timeout {
            let expired = format!("event:process{number}.timeout");
            b.node(
                &expired,
                NodeKind::Event,
                "process timeout",
                Some(process.span),
                Vec::new(),
                Vec::new(),
            );
            let edge = b.edge(
                &id,
                &expired,
                EdgeKind::Timeout,
                Some(format!("after {}", duration(timeout.millis))),
            );
            edge.asynchronous = true;
            edge.security_significant = true;
            walk_behavior(
                b,
                s,
                number,
                &mut counters,
                &process.on_timeout,
                &expired,
                Some("on timeout"),
            );
        }
    }
    for barrier in &s.barriers {
        let id = b.barrier(barrier);
        for member in &barrier.members {
            let from = b.value(s, member);
            let edge = b.edge(&from, &id, EdgeKind::Join, None);
            edge.asynchronous = true;
            edge.generation_binding = Some(JOIN_KEY.to_owned());
        }
    }
}

fn event_node(b: &mut Builder, event: &str) -> String {
    let id = format!("event:{event}");
    b.node(&id, NodeKind::Event, event, None, Vec::new(), Vec::new());
    id
}

fn walk_behavior(
    b: &mut Builder,
    s: &Symbols,
    process: usize,
    counters: &mut Counters,
    statements: &[SequentialStatement],
    anchor: &str,
    condition: Option<&str>,
) {
    let control = |b: &mut Builder, to: &str, kind: EdgeKind, extra: Option<String>| {
        let label = match (condition, extra) {
            (Some(condition), Some(extra)) => Some(format!("[{condition}] {extra}")),
            (Some(condition), None) => Some(format!("[{condition}]")),
            (None, extra) => extra,
        };
        let edge = b.edge(anchor, to, kind, label);
        edge.condition = condition.map(|condition| format!("[{condition}]"));
    };
    for statement in statements {
        match statement {
            SequentialStatement::DeviceCall(call) => {
                counters.calls += 1;
                let invocation = invocation_node(b, process, counters.calls, call);
                control(b, &invocation, EdgeKind::Call, None);
                let output = b.value(s, &call.output);
                b.edge(&invocation, &output, EdgeKind::Result, None)
                    .asynchronous = true;
                outcomes(b, s, call, &invocation);
            }
            SequentialStatement::Assignment(assignment) => {
                let to = b.value(s, &assignment.target);
                let detail = b
                    .detail()
                    .then(|| format!("<= {}", clip(&assignment.value.source)));
                control(b, &to, EdgeKind::DataFlow, detail);
            }
            SequentialStatement::If(branching) => {
                counters.decisions += 1;
                let id = format!("decision:p{process}.if{}", counters.decisions);
                b.node(
                    &id,
                    NodeKind::Decision,
                    "if",
                    Some(branching.span),
                    Vec::new(),
                    Vec::new(),
                );
                control(b, &id, EdgeKind::Control, None);
                for branch in &branching.branches {
                    let text = clip(&branch.condition.source);
                    walk_behavior(
                        b,
                        s,
                        process,
                        counters,
                        &branch.statements,
                        &id,
                        Some(&text),
                    );
                }
                walk_behavior(
                    b,
                    s,
                    process,
                    counters,
                    &branching.otherwise,
                    &id,
                    Some("else"),
                );
            }
            SequentialStatement::Parallel(parallel) => {
                counters.forks += 1;
                let fork = format!("fork:p{process}.par{}", counters.forks);
                let join = format!("{fork}.join");
                b.node(
                    &fork,
                    NodeKind::Fork,
                    "parallel",
                    Some(parallel.span),
                    Vec::new(),
                    Vec::new(),
                );
                b.node(
                    &join,
                    NodeKind::Fork,
                    "commit together",
                    Some(parallel.span),
                    Vec::new(),
                    Vec::new(),
                )
                .generation_sensitive = true;
                control(b, &fork, EdgeKind::Control, None);
                for call in &parallel.calls {
                    counters.calls += 1;
                    let invocation = invocation_node(b, process, counters.calls, call);
                    b.edge(&fork, &invocation, EdgeKind::Fork, None);
                    let edge = b.edge(&invocation, &join, EdgeKind::Join, None);
                    edge.asynchronous = true;
                    edge.generation_binding = Some(JOIN_KEY.to_owned());
                    let output = b.value(s, &call.output);
                    b.edge(&join, &output, EdgeKind::Result, None);
                    outcomes(b, s, call, &invocation);
                }
            }
            SequentialStatement::Assert {
                condition: asserted,
                span,
            } => {
                if b.options.show_policies {
                    counters.asserts += 1;
                    let id = format!("policy:p{process}.assert{}", counters.asserts);
                    b.node(
                        &id,
                        NodeKind::Policy,
                        &format!("assert {}", clip(&asserted.source)),
                        Some(*span),
                        Vec::new(),
                        Vec::new(),
                    );
                    control(b, &id, EdgeKind::Control, None);
                }
            }
            SequentialStatement::Null { .. } => {}
        }
    }
}

fn invocation_node(b: &mut Builder, process: usize, number: usize, call: &DeviceCall) -> String {
    let id = format!("invocation:p{process}.call{number}");
    let significant = call
        .timeout
        .map(|timeout| vec![format!("timeout {}", duration(timeout.millis))])
        .unwrap_or_default();
    let node = b.node(
        &id,
        NodeKind::Invocation,
        &format!("{}.{}", call.device, call.method),
        Some(call.span),
        vec!["async call".to_owned()],
        significant,
    );
    node.generation_sensitive = true;
    node.metadata
        .insert("device".to_owned(), call.device.clone());
    node.metadata
        .insert("method".to_owned(), call.method.clone());
    id
}

/// `.done` / `.failed` / `.timeout` events raised by a call, when a process
/// reacts to them (or always with `--show-internal`).
fn outcomes(b: &mut Builder, s: &Symbols, call: &DeviceCall, invocation: &str) {
    for outcome in ["done", "failed", "timeout"] {
        if outcome == "timeout" && call.timeout.is_none() {
            continue;
        }
        if !(s.subscribed(&call.device, outcome) || b.options.show_internal) {
            continue;
        }
        let event = event_node(b, &format!("{}.{outcome}", call.device));
        let kind = if outcome == "timeout" {
            EdgeKind::Timeout
        } else {
            EdgeKind::Result
        };
        let edge = b.edge(invocation, &event, kind, Some(outcome.to_owned()));
        edge.asynchronous = true;
        // Timeouts stay explicit even in compact mode.
        edge.security_significant |= kind == EdgeKind::Timeout;
    }
}

// ---------------------------------------------------------------------------
// Security view

fn security(b: &mut Builder, s: &Symbols) {
    b.group("zone:local", "zone", "local zone", None);
    b.group(
        "orchestrator",
        "orchestrator",
        "AI Conductor orchestrator (signal store)",
        Some("zone:local"),
    );
    // The signal store is split by class, strongest first, so that renderers
    // can draw a rule that denies a whole class as one edge.
    for class in [Class::Restricted, Class::Internal, Class::Public] {
        let group = format!("class:{}", class_name(class));
        for name in &s.value_order {
            if s.class(name) != Some(class) {
                continue;
            }
            b.group(&group, "class", class_name(class), Some("orchestrator"));
            let id = b.value(s, name);
            b.nodes[b.node_index[&id]].group = Some(group.clone());
        }
    }
    for device in &s.devices {
        b.device(device, None);
    }

    for process in &s.processes {
        for statement in process_statements(process) {
            for call in calls_of(statement) {
                let Some(device) = s.device(&call.device) else {
                    continue;
                };
                let device_id = b.device(device, None);
                let human = device.device_type == "human";
                let cloud = location_of(device) == Location::Cloud;
                for read in s.call_reads(call) {
                    let from = b.value(s, read);
                    let class = s.class(read).map(class_name).unwrap_or("public");
                    let kind = if human {
                        EdgeKind::ApprovalGate
                    } else {
                        EdgeKind::SecurityFlow
                    };
                    let label = if human {
                        format!("{class} · human approval")
                    } else {
                        class.to_owned()
                    };
                    let edge = b.edge(&from, &device_id, kind, Some(label));
                    edge.classification = Some(class.to_owned());
                    edge.metadata
                        .insert("state".to_owned(), "allowed".to_owned());
                    if human {
                        edge.guarded = true;
                        edge.security_significant = true;
                    }
                    if cloud {
                        mark_cloud_egress(edge);
                        edge.metadata
                            .insert("state".to_owned(), "guarded".to_owned());
                    }
                }
                let to = b.value(s, &call.output);
                let class = s.class(&call.output).map(class_name).unwrap_or("public");
                let edge = b.edge(
                    &device_id,
                    &to,
                    EdgeKind::SecurityFlow,
                    Some(class.to_owned()),
                );
                edge.classification = Some(class.to_owned());
                edge.metadata
                    .insert("state".to_owned(), "allowed".to_owned());
                if cloud {
                    edge.security_significant = true;
                    edge.metadata
                        .insert("crossing".to_owned(), "cloud -> local".to_owned());
                }
            }
            if let SequentialStatement::Assignment(assignment) = statement {
                let to = b.value(s, &assignment.target);
                for read in s.reads(assignment.value.expr.names()) {
                    let from = b.value(s, read);
                    let class = s.class(read).map(class_name).unwrap_or("public");
                    let edge = b.edge(&from, &to, EdgeKind::SecurityFlow, Some(class.to_owned()));
                    edge.classification = Some(class.to_owned());
                    edge.metadata
                        .insert("state".to_owned(), "allowed".to_owned());
                }
            }
        }
    }

    // Flows the checker forbids are drawn as denied, never as ordinary edges.
    let (floor, sources) = s.cloud_floor();
    let rule = "policy:never.cloud";
    let mut label = vec![format!("{}+ -> cloud", class_name(floor))];
    if b.detail() {
        label.extend(sources);
    }
    let node = b.node(rule, NodeKind::Policy, "DENIED", None, Vec::new(), label);
    node.metadata
        .insert("floor".to_owned(), class_name(floor).to_owned());
    let blocked = s
        .value_order
        .iter()
        .filter(|name| s.class(name).is_some_and(|class| class >= floor))
        .collect::<Vec<_>>();
    let clouds = s
        .devices
        .iter()
        .filter(|device| location_of(device) == Location::Cloud)
        .collect::<Vec<_>>();
    for name in &blocked {
        let from = b.value(s, name);
        let class = s.class(name).map(class_name).unwrap_or("public");
        denied(b, &from, rule, Some(class.to_owned()));
    }
    for device in &clouds {
        let to = b.device(device, None);
        denied(b, rule, &to, None);
    }
    for device in &s.devices {
        let Some(clearance) = device
            .generic("clearance")
            .and_then(GenericValue::as_ident)
            .and_then(parse_class)
        else {
            continue;
        };
        let cloud = location_of(device) == Location::Cloud;
        let to = b.device(device, None);
        for name in &s.value_order {
            let Some(class) = s.class(name) else { continue };
            if class > clearance && !(cloud && class >= floor) {
                let from = b.value(s, name);
                denied(
                    b,
                    &from,
                    &to,
                    Some(format!(
                        "{} exceeds clearance {}",
                        class_name(class),
                        class_name(clearance)
                    )),
                );
            }
        }
    }
    b.annotate(
        "v0.1 profile: no declassifier or gateway device; a classification is never lowered. \
         Derived data keeps the strongest class of its inputs.",
    );
}

fn denied(b: &mut Builder, from: &str, to: &str, detail: Option<String>) {
    let label = match detail {
        Some(detail) => format!("DENIED · {detail}"),
        None => "DENIED".to_owned(),
    };
    // A denied flow out of a value carries that value's class.
    let class = b
        .node_index
        .get(from)
        .and_then(|&index| b.nodes[index].classification.clone());
    let edge = b.edge(from, to, EdgeKind::SecurityFlow, Some(label));
    edge.classification = class;
    edge.denied = true;
    edge.security_significant = true;
    edge.metadata
        .insert("state".to_owned(), "denied".to_owned());
}

// ---------------------------------------------------------------------------
// Policies (--show-policies)

fn policies(b: &mut Builder, s: &Symbols) {
    for declaration in &s.architecture.declarations {
        if let awhdl_ast::Declaration::Budget(budget) = declaration {
            let limits = budget
                .limits
                .iter()
                .map(|limit| match limit.value {
                    BudgetValue::Count(count) => format!("{} <= {count}", limit.key),
                    BudgetValue::Time(time) => {
                        format!("{} <= {}", limit.key, duration(time.millis))
                    }
                })
                .collect();
            b.node(
                &format!("policy:budget.{}", budget.name),
                NodeKind::Policy,
                &format!("budget {}", budget.name),
                Some(budget.span),
                limits,
                Vec::new(),
            );
        }
    }
    let mut number = 0;
    for statement in &s.architecture.statements {
        let ConcurrentStatement::Assert(assert) = statement else {
            continue;
        };
        number += 1;
        let text = match &assert.assertion {
            Assertion::Always { condition } => {
                format!("assert always ({})", clip(&condition.source))
            }
            Assertion::Never { condition } => format!("assert never ({})", clip(&condition.source)),
            Assertion::NeverFlow {
                classification,
                location,
            } => {
                if b.view == View::Security {
                    continue;
                }
                format!("assert never ({classification} -> {location})")
            }
        };
        b.node(
            &format!("policy:assert{number}"),
            NodeKind::Policy,
            &text,
            Some(assert.span),
            Vec::new(),
            Vec::new(),
        );
    }
}

// ---------------------------------------------------------------------------
// Text helpers

fn kind_name(kind: EdgeKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn class_name(class: Class) -> &'static str {
    match class {
        Class::Public => "public",
        Class::Internal => "internal",
        Class::Restricted => "restricted",
    }
}

fn location_name(location: Location) -> &'static str {
    match location {
        Location::Local => "local",
        Location::Cloud => "cloud",
    }
}

fn mode_name(mode: PortMode) -> &'static str {
    match mode {
        PortMode::In => "in",
        PortMode::Out => "out",
        PortMode::Inout => "inout",
    }
}

fn generic_text(value: &GenericValue) -> String {
    match value {
        GenericValue::Ident(value) | GenericValue::String(value) => value.clone(),
        GenericValue::Integer(value) => value.to_string(),
        GenericValue::Time(time) => duration(time.millis),
    }
}

fn duration(millis: u64) -> String {
    for (unit, size) in [
        ("day", 86_400_000),
        ("hour", 3_600_000),
        ("min", 60_000),
        ("sec", 1_000),
    ] {
        if millis >= size && millis % size == 0 {
            return format!("{} {unit}", millis / size);
        }
    }
    format!("{millis} ms")
}

/// Source text shortened for a label; renderers still escape it.
fn clip(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= 48 {
        flat
    } else {
        format!("{}...", flat.chars().take(45).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use crate::{EdgeKind, Format, Graph, NodeKind, Options, View, project, render};
    use awhdl_parser::parse;

    const SECURE: &str = include_str!("../../../tests/graph/secure_development.awhdl");

    fn graph(source: &str, view: View, options: &Options) -> Graph {
        let design = parse(source).unwrap();
        assert!(awhdl_checker::check(&design).is_empty());
        project(&design, source, view, options).unwrap()
    }

    fn node<'g>(graph: &'g Graph, id: &str) -> &'g crate::Node {
        graph
            .nodes
            .iter()
            .find(|node| node.id == id)
            .unwrap_or_else(|| panic!("no node {id}"))
    }

    #[test]
    fn structure_places_devices_in_their_zones() {
        let graph = graph(SECURE, View::Structure, &Options::default());
        assert_eq!(
            node(&graph, "device:local_coder").group.as_deref(),
            Some("zone:local")
        );
        assert_eq!(
            node(&graph, "device:cloud_reviewer").group.as_deref(),
            Some("zone:cloud")
        );
        assert_eq!(node(&graph, "device:operator").kind, NodeKind::Approval);
        // Ports sit on the boundary, signals inside the entity.
        assert_eq!(node(&graph, "value:task").group, None);
        assert_eq!(
            node(&graph, "value:candidate").group.as_deref(),
            Some("entity:secure_development")
        );
        let egress = graph
            .edges
            .iter()
            .find(|edge| edge.to == "device:cloud_reviewer")
            .unwrap();
        assert!(egress.guarded && egress.security_significant);
    }

    #[test]
    fn security_view_lets_only_cleared_data_reach_the_cloud() {
        let graph = graph(SECURE, View::Security, &Options::default());
        let into_cloud = graph
            .edges
            .iter()
            .filter(|edge| edge.to == "device:cloud_reviewer")
            .collect::<Vec<_>>();
        let allowed = into_cloud
            .iter()
            .filter(|edge| !edge.denied)
            .map(|edge| edge.from.as_str())
            .collect::<Vec<_>>();
        assert_eq!(allowed, ["value:note"]);
        assert!(into_cloud.iter().any(|edge| edge.denied));
        // `assert never (internal -> cloud)` lowers the floor to internal.
        let rule = node(&graph, "policy:never.cloud");
        assert_eq!(rule.metadata["floor"], "internal");
        for name in ["task", "source", "candidate", "review"] {
            let edge = graph
                .edges
                .iter()
                .find(|edge| edge.from == format!("value:{name}") && edge.to == rule.id)
                .unwrap_or_else(|| panic!("{name} is not shown as denied"));
            assert!(edge.denied && edge.label.as_deref().unwrap().starts_with("DENIED"));
        }
        assert!(
            !graph
                .edges
                .iter()
                .any(|edge| edge.from == "value:note" && edge.denied)
        );
        // Human approval is a guarded gate.
        let approval = graph
            .edges
            .iter()
            .find(|edge| edge.to == "device:operator")
            .unwrap();
        assert!(approval.kind == EdgeKind::ApprovalGate && approval.guarded);
    }

    #[test]
    fn behavior_view_shows_forks_generation_aware_joins_and_the_retry_loop() {
        let graph = graph(SECURE, View::Behavior, &Options::default());
        let forked = graph
            .edges
            .iter()
            .filter(|edge| edge.kind == EdgeKind::Fork)
            .count();
        assert_eq!(forked, 2);
        let joins = graph
            .edges
            .iter()
            .filter(|edge| edge.kind == EdgeKind::Join)
            .collect::<Vec<_>>();
        assert_eq!(
            joins.len(),
            4,
            "two parallel branches and two barrier members"
        );
        assert!(joins.iter().all(|edge| edge.generation_binding.is_some()));
        assert!(
            graph
                .edges
                .iter()
                .any(|edge| edge.kind == EdgeKind::Retry && edge.to == "value:candidate")
        );
        let timeout = node(&graph, "invocation:p1.call1");
        assert!(timeout.label.iter().any(|line| line == "timeout 5 min"));
        // `process(local_coder.timeout)` observes the call's timeout.
        assert!(graph.edges.iter().any(|edge| edge.kind == EdgeKind::Timeout
            && edge.from == "invocation:p1.call1"
            && edge.to == "event:local_coder.timeout"));
        // compiler.failed is observed, tester.failed is not.
        node(&graph, "event:compiler.failed");
        assert!(
            !graph
                .nodes
                .iter()
                .any(|node| node.id == "event:tester.failed")
        );
        let internal = Options {
            show_internal: true,
            ..Options::default()
        };
        let all = self::graph(SECURE, View::Behavior, &internal);
        node(&all, "event:tester.failed");
    }

    #[test]
    fn petri_view_is_a_place_transition_net() {
        let graph = graph(SECURE, View::Petri, &Options::default());
        let kind = |id: &str| node(&graph, id).kind;
        assert!(
            graph
                .nodes
                .iter()
                .all(|node| matches!(node.kind, NodeKind::Place | NodeKind::Transition))
        );
        for edge in &graph.edges {
            assert_ne!(
                kind(&edge.from),
                kind(&edge.to),
                "{} is not bipartite",
                edge.id
            );
        }
        // The inputs wake process 1 when the run starts, and nothing else.
        let marked = graph
            .nodes
            .iter()
            .filter(|node| node.metadata.contains_key("tokens"))
            .map(|node| node.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(marked, ["place:p1.wake"]);
        let into = |place: &str| {
            graph
                .edges
                .iter()
                .filter(|edge| edge.to == place)
                .map(|edge| edge.from.as_str())
                .collect::<Vec<_>>()
        };
        // Any one of heartbeat or compiler.failed wakes process 5: one place, two producers.
        let woken = into("place:p5.wake");
        assert!(woken.contains(&"transition:timer.heartbeat"));
        assert!(woken.contains(&"transition:p2.s1.call1.failed"));
        // A call timeout raises local_coder.timeout (process 6).
        assert!(into("place:p6.wake").contains(&"transition:p1.s1.timeout"));
        // A broadcast is explicit: every changed write of status reaches process 4.
        assert_eq!(into("place:p4.wake").len(), 3);
        // Barrier members and the parallel join are generation-aware.
        assert!(node(&graph, "transition:barrier.validated").generation_sensitive);
        assert!(node(&graph, "transition:p2.s1.join").generation_sensitive);
        assert!(
            graph
                .edges
                .iter()
                .filter(|edge| edge.to == "transition:p2.s1.join")
                .all(|edge| edge.generation_binding.is_some())
        );
    }

    #[test]
    fn activity_view_has_one_structured_activity_per_process_and_barrier() {
        let graph = graph(SECURE, View::Activity, &Options::default());
        let activities = graph
            .groups
            .iter()
            .filter(|group| group.kind == "activity")
            .collect::<Vec<_>>();
        assert_eq!(activities.len(), 7, "six processes and one barrier");
        for activity in &activities {
            for kind in [NodeKind::Initial, NodeKind::Final] {
                let count = graph
                    .nodes
                    .iter()
                    .filter(|node| node.kind == kind && node.group.as_deref() == Some(&activity.id))
                    .count();
                assert_eq!(count, 1, "{kind:?} in {}", activity.id);
            }
        }
        // Every decision names its merge and every fork its join.
        for node in &graph.nodes {
            let key = match node.kind {
                NodeKind::Decision => "merge",
                NodeKind::Fork => "join",
                _ => continue,
            };
            let closer = &node.metadata[key];
            assert!(graph.nodes.iter().any(|other| &other.id == closer));
        }
        let accept = node(&graph, "activity:p5.accept");
        assert_eq!(accept.label, ["heartbeat (every 30 sec) | compiler.failed"]);
        // Signals are sent only for events something observes.
        let sent = graph
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::SendSignal)
            .map(|node| node.label[0].as_str())
            .collect::<std::collections::BTreeSet<_>>();
        for signal in [
            "candidate changed",
            "local_coder.timeout",
            "compiler.failed",
            "validated.ready",
        ] {
            assert!(sent.contains(signal), "{signal} not sent");
        }
        assert!(!sent.contains("review changed") && !sent.contains("tester.failed"));
    }

    #[test]
    fn the_state_view_does_not_invent_states() {
        let design = parse(SECURE).unwrap();
        let error = project(&design, SECURE, View::State, &Options::default()).unwrap_err();
        assert!(error.0.contains("no workflow states"));
    }

    #[test]
    fn several_architectures_need_a_choice() {
        let two =
            format!("{SECURE}\narchitecture other of secure_development is\nbegin\nend other;\n");
        let design = parse(&two).unwrap();
        let error = project(&design, &two, View::Structure, &Options::default()).unwrap_err();
        assert!(error.0.contains("--architecture"));
        let chosen = Options {
            architecture: Some("other".to_owned()),
            ..Options::default()
        };
        let graph = project(&design, &two, View::Structure, &chosen).unwrap();
        assert_eq!(graph.architecture, "other");
    }

    #[test]
    fn compact_mode_keeps_security_significant_labels() {
        let compact = Options {
            compact: true,
            ..Options::default()
        };
        let graph = graph(SECURE, View::Security, &compact);
        assert!(graph.nodes.iter().all(|node| node.label.len() == 1));
        assert!(
            graph
                .edges
                .iter()
                .filter(|edge| edge.denied || edge.guarded)
                .all(|edge| edge.label.is_some())
        );
        let restricted = graph
            .edges
            .iter()
            .find(|edge| edge.from == "value:source" && edge.to == "device:local_coder")
            .unwrap();
        assert_eq!(restricted.label.as_deref(), Some("restricted"));
        let full = self::graph(SECURE, View::Security, &Options::default());
        assert_eq!(graph.edges.len(), full.edges.len());
        let behavior = self::graph(SECURE, View::Behavior, &compact);
        assert!(
            behavior
                .edges
                .iter()
                .filter(|edge| edge.kind == EdgeKind::Timeout)
                .all(|edge| edge.label.is_some())
        );
    }

    #[test]
    fn rendering_is_deterministic_and_escapes_source_text() {
        let hostile = SECURE.replace(
            "\"coder timed out\"",
            "\"x\\\"] --> evil[\\\"pwn | %% <br/>\"",
        );
        let graph = graph(&hostile, View::Behavior, &Options::default());
        for format in [Format::Mermaid, Format::Dot, Format::Json] {
            assert_eq!(render(&graph, format), render(&graph, format));
            assert!(render(&graph, format).is_ok());
        }
        let mermaid = render(&graph, Format::Mermaid).unwrap();
        assert!(!mermaid.contains("evil["));
        assert!(!mermaid.contains("pwn |") && !mermaid.contains("%% <br/>"));
        assert!(mermaid.contains("evil#91;"));
    }
}
