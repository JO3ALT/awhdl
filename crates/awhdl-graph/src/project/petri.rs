//! Petri view: a place/transition net with data abstracted.
//!
//! Places hold control tokens: a process that has been woken, a point between
//! statements, a call that is running, a barrier member that has changed.
//! Transitions are the steps of the design. Branch guards and whether a write
//! changes a value are abstracted into nondeterministic choices, so every run
//! of the design is a firing sequence of the net (an over-approximation).
use super::{Builder, JOIN_KEY, Symbols, Trigger, clip, duration, trigger};
use crate::{EdgeKind, NodeKind};
use awhdl_ast::{DeviceCall, PortMode, SequentialStatement, Span};
use std::collections::{BTreeMap, BTreeSet};

pub(super) const ABSTRACTION: &str = "Place/transition net with data abstracted: branch guards and \
     whether a write changes a value are nondeterministic choices, and delta cycles are \
     interleaved. Every run of the design is a firing sequence of the net, not the reverse: \
     use it for reachability and safety, not for termination.";

/// Which processes each runtime event wakes, and which barriers a value feeds.
struct Wiring<'s> {
    /// Event key (`x`, `t`, `b`, `d.done`, ...) -> woken process numbers.
    wakes: BTreeMap<String, BTreeSet<usize>>,
    /// Value name -> barriers it is a member of.
    barriers: BTreeMap<&'s str, Vec<&'s str>>,
}

/// The event a sensitivity name stands for: `x` and `x.changed` are one
/// event, as are `b` and `b.ready`.
pub(super) fn event_key(s: &Symbols, text: &str) -> String {
    match trigger(s, text) {
        Trigger::Value(name) => name.to_owned(),
        Trigger::Timer(timer) => timer.name.clone(),
        Trigger::Barrier(barrier) => barrier.name.clone(),
        Trigger::Event(event) => event.to_owned(),
    }
}

impl<'s> Wiring<'s> {
    fn new(s: &Symbols<'s>) -> Self {
        let mut wakes: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
        for (index, process) in s.processes.iter().enumerate() {
            for text in &process.sensitivity {
                wakes
                    .entry(event_key(s, text))
                    .or_default()
                    .insert(index + 1);
            }
        }
        let mut barriers: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for barrier in &s.barriers {
            for member in &barrier.members {
                barriers.entry(member).or_default().push(&barrier.name);
            }
        }
        Wiring { wakes, barriers }
    }

    /// A write to `name` is observable (some process or barrier sees it).
    fn observed(&self, name: &str) -> bool {
        self.wakes.contains_key(name) || self.barriers.contains_key(name)
    }
}

pub(super) fn petri(b: &mut Builder, s: &Symbols) {
    b.annotate(ABSTRACTION);
    let wiring = Wiring::new(s);
    let mut net = Net { b, s, wiring };
    for number in 1..=s.processes.len() {
        net.wake_place(number);
    }
    // Inputs are given when the run starts: their processes begin woken.
    for name in &s.value_order {
        let input = matches!(
            s.values[name].port,
            Some(PortMode::In) | Some(PortMode::Inout)
        );
        if !input {
            continue;
        }
        for &number in net.wiring.wakes.get(*name).into_iter().flatten() {
            let id = format!("place:p{number}.wake");
            net.b.nodes[net.b.node_index[&id]]
                .metadata
                .insert("tokens".to_owned(), "1".to_owned());
        }
    }
    for timer in &s.timers {
        let id = format!("transition:timer.{}", timer.name);
        net.transition(
            &id,
            &format!("{} ticks", timer.name),
            Some(timer.span),
            vec![format!("every {}", duration(timer.period.millis))],
        );
        net.emit(&id, &timer.name);
    }
    for barrier in &s.barriers {
        let id = format!("transition:barrier.{}", barrier.name);
        net.transition(
            &id,
            &format!("{} ready", barrier.name),
            Some(barrier.span),
            Vec::new(),
        );
        net.b.nodes[net.b.node_index[&id]].generation_sensitive = true;
        for member in &barrier.members {
            let place = net.member_place(&barrier.name, member);
            net.b
                .edge(&place, &id, EdgeKind::Join, None)
                .generation_binding = Some(JOIN_KEY.to_owned());
        }
        net.emit(&id, &barrier.name);
    }
    for (index, process) in s.processes.iter().enumerate() {
        net.process(index + 1, process);
    }
    if !s.timers.is_empty() {
        net.b.annotate(
            "Timers are transitions without input places, so the net is unbounded; budgets are not modelled.",
        );
    }
}

struct Net<'b, 'a, 's> {
    b: &'b mut Builder<'a>,
    s: &'b Symbols<'s>,
    wiring: Wiring<'s>,
}

impl Net<'_, '_, '_> {
    fn place(&mut self, id: &str, label: &str, span: Option<Span>) -> String {
        self.b
            .node(id, NodeKind::Place, label, span, Vec::new(), Vec::new());
        id.to_owned()
    }

    fn transition(&mut self, id: &str, label: &str, span: Option<Span>, significant: Vec<String>) {
        self.b.node(
            id,
            NodeKind::Transition,
            label,
            span,
            Vec::new(),
            significant,
        );
    }

    fn arc(&mut self, from: &str, to: &str, kind: EdgeKind) -> &mut crate::Edge {
        self.b.edge(from, to, kind, None)
    }

    fn wake_place(&mut self, number: usize) -> String {
        let process = self.s.processes[number - 1];
        self.place(
            &format!("place:p{number}.wake"),
            &format!("p{number} woken"),
            Some(process.span),
        )
    }

    fn member_place(&mut self, barrier: &str, member: &str) -> String {
        let id = format!("place:barrier.{barrier}.{member}");
        self.place(&id, &format!("{barrier} has {member}"), None);
        self.b.nodes[self.b.node_index[&id]].generation_sensitive = true;
        id
    }

    /// Arcs from a transition to every place its event wakes.
    fn emit(&mut self, transition: &str, event: &str) {
        let woken = self.wiring.wakes.get(event).cloned().unwrap_or_default();
        for number in woken {
            let place = format!("place:p{number}.wake");
            self.b
                .edge(transition, &place, EdgeKind::EventTrigger, None)
                .asynchronous = true;
        }
    }

    /// A write that changed `name` wakes its processes and feeds its barriers.
    fn emit_change(&mut self, transition: &str, name: &str) {
        self.emit(transition, name);
        let barriers = self.wiring.barriers.get(name).cloned().unwrap_or_default();
        for barrier in barriers {
            let place = self.member_place(barrier, name);
            self.arc(transition, &place, EdgeKind::DataFlow);
        }
    }

    fn process(&mut self, number: usize, process: &awhdl_ast::ProcessStmt) {
        let name = format!("process({})", process.sensitivity.join(", "));
        let wake = format!("place:p{number}.wake");
        let start = format!("transition:p{number}.start");
        let done = self.place(
            &format!("place:p{number}.done"),
            &format!("p{number} done"),
            Some(process.span),
        );
        self.transition(&start, &name, Some(process.span), Vec::new());
        self.arc(&wake, &start, EdgeKind::EventTrigger);
        let mut steps = Steps {
            process: number,
            ..Steps::default()
        };
        match process.timeout {
            None => {
                let first = self.control(&mut steps);
                self.arc(&start, &first, EdgeKind::Control);
                self.walk(&mut steps, &process.statements, &first, &done);
            }
            Some(limit) => {
                // The body either finishes in time or is discarded for `on timeout`.
                let armed = self.control(&mut steps);
                self.arc(&start, &armed, EdgeKind::Control);
                let body = format!("transition:p{number}.body");
                let expired = format!("transition:p{number}.timeout");
                self.transition(&body, "body in time", Some(process.span), Vec::new());
                self.transition(
                    &expired,
                    "process timeout",
                    Some(process.span),
                    vec![format!("after {}", duration(limit.millis))],
                );
                self.arc(&armed, &body, EdgeKind::Control);
                self.arc(&armed, &expired, EdgeKind::Timeout)
                    .security_significant = true;
                let body_start = self.control(&mut steps);
                self.arc(&body, &body_start, EdgeKind::Control);
                self.walk(&mut steps, &process.statements, &body_start, &done);
                let handler = self.control(&mut steps);
                self.arc(&expired, &handler, EdgeKind::Control);
                self.walk(&mut steps, &process.on_timeout, &handler, &done);
            }
        }
        let end = format!("transition:p{number}.end");
        self.transition(
            &end,
            &format!("{name} ends"),
            Some(process.span),
            Vec::new(),
        );
        self.arc(&done, &end, EdgeKind::Control);
    }

    /// A fresh control place of the current process.
    fn control(&mut self, steps: &mut Steps) -> String {
        steps.places += 1;
        let id = format!("place:p{}.c{}", steps.process, steps.places);
        self.place(&id, &format!("p{}.{}", steps.process, steps.places), None);
        id
    }

    /// Statements in sequence from place `from` to place `to`.
    fn walk(
        &mut self,
        steps: &mut Steps,
        statements: &[SequentialStatement],
        from: &str,
        to: &str,
    ) {
        let effective = statements
            .iter()
            .filter(|statement| {
                !matches!(
                    statement,
                    SequentialStatement::Null { .. } | SequentialStatement::Assert { .. }
                )
            })
            .collect::<Vec<_>>();
        if effective.is_empty() {
            steps.steps += 1;
            let id = format!("transition:p{}.s{}.skip", steps.process, steps.steps);
            self.transition(&id, "no effect", None, Vec::new());
            self.arc(from, &id, EdgeKind::Control);
            self.arc(&id, to, EdgeKind::Control);
            return;
        }
        let mut current = from.to_owned();
        for (index, statement) in effective.iter().enumerate() {
            let next = if index + 1 == effective.len() {
                to.to_owned()
            } else {
                self.control(steps)
            };
            self.statement(steps, statement, &current, &next);
            current = next;
        }
    }

    fn statement(
        &mut self,
        steps: &mut Steps,
        statement: &SequentialStatement,
        from: &str,
        to: &str,
    ) {
        steps.steps += 1;
        let base = format!("transition:p{}.s{}", steps.process, steps.steps);
        match statement {
            SequentialStatement::Assignment(assignment) => {
                let text = format!(
                    "{} <= {}",
                    assignment.target,
                    clip(&assignment.value.source)
                );
                self.write(
                    &base,
                    &text,
                    &assignment.target,
                    Some(assignment.span),
                    from,
                    to,
                );
            }
            SequentialStatement::DeviceCall(call) => {
                self.call_outcomes(&base, call, from, to, true);
            }
            SequentialStatement::If(branching) => {
                let mut arms = branching
                    .branches
                    .iter()
                    .map(|branch| {
                        (
                            format!("[{}]", clip(&branch.condition.source)),
                            &branch.statements[..],
                        )
                    })
                    .collect::<Vec<_>>();
                arms.push((
                    if branching.otherwise.is_empty() {
                        "[no branch taken]".to_owned()
                    } else {
                        "[else]".to_owned()
                    },
                    &branching.otherwise[..],
                ));
                for (k, (condition, body)) in arms.into_iter().enumerate() {
                    let arm = format!("{base}.b{}", k + 1);
                    self.transition(&arm, &condition, Some(branching.span), Vec::new());
                    self.arc(from, &arm, EdgeKind::Control).condition = Some(condition);
                    if body.is_empty() {
                        self.arc(&arm, to, EdgeKind::Control);
                    } else {
                        let entry = self.control(steps);
                        self.arc(&arm, &entry, EdgeKind::Control);
                        self.walk(steps, body, &entry, to);
                    }
                }
            }
            SequentialStatement::Parallel(parallel) => {
                let fork = format!("{base}.fork");
                let join = format!("{base}.join");
                self.transition(&fork, "parallel", Some(parallel.span), Vec::new());
                self.transition(&join, "commit together", Some(parallel.span), Vec::new());
                self.b.nodes[self.b.node_index[&join]].generation_sensitive = true;
                self.arc(from, &fork, EdgeKind::Control);
                for (j, call) in parallel.calls.iter().enumerate() {
                    let branch = format!("place:p{}.s{}.call{}", steps.process, steps.steps, j + 1);
                    let name = format!("{}.{}", call.device, call.method);
                    let running = self.place(
                        &format!("{branch}.running"),
                        &format!("{name} running"),
                        Some(call.span),
                    );
                    let finished = self.place(
                        &format!("{branch}.finished"),
                        &format!("{name} finished"),
                        Some(call.span),
                    );
                    self.b.nodes[self.b.node_index[&finished]].generation_sensitive = true;
                    self.arc(&fork, &running, EdgeKind::Fork);
                    let outcomes = format!("{base}.call{}", j + 1);
                    self.call_outcomes(&outcomes, call, &running, &finished, false);
                    self.arc(&finished, &join, EdgeKind::Join)
                        .generation_binding = Some(JOIN_KEY.to_owned());
                }
                // Results commit together after the join, one output at a time.
                let mut current = self.control(steps);
                self.arc(&join, &current, EdgeKind::Control);
                for (j, call) in parallel.calls.iter().enumerate() {
                    let next = if j + 1 == parallel.calls.len() {
                        to.to_owned()
                    } else {
                        self.control(steps)
                    };
                    let text = format!("{} <= {}.{} result", call.output, call.device, call.method);
                    self.write(
                        &format!("{base}.commit{}", j + 1),
                        &text,
                        &call.output,
                        Some(call.span),
                        &current,
                        &next,
                    );
                    current = next;
                }
            }
            SequentialStatement::Assert { .. } | SequentialStatement::Null { .. } => {
                self.arc(from, to, EdgeKind::Control);
            }
        }
    }

    /// A write: split into "changed" and "unchanged" when anything observes it.
    fn write(
        &mut self,
        base: &str,
        text: &str,
        target: &str,
        span: Option<Span>,
        from: &str,
        to: &str,
    ) {
        if !self.wiring.observed(target) {
            let id = format!("{base}.write");
            self.transition(&id, text, span, Vec::new());
            self.arc(from, &id, EdgeKind::Control);
            self.arc(&id, to, EdgeKind::DataFlow);
            return;
        }
        for changed in [true, false] {
            let id = format!("{base}.{}", if changed { "changed" } else { "unchanged" });
            let label = format!("{text} · {}", if changed { "changed" } else { "unchanged" });
            self.transition(&id, &label, span, Vec::new());
            self.arc(from, &id, EdgeKind::Control);
            self.arc(&id, to, EdgeKind::DataFlow);
            if changed {
                self.emit_change(&id, target);
            }
        }
    }

    /// Competing outcomes of one call. A sequential call writes its output on
    /// success; a parallel one writes after the join (`write_output = false`).
    fn call_outcomes(
        &mut self,
        base: &str,
        call: &DeviceCall,
        from: &str,
        to: &str,
        write_output: bool,
    ) {
        let name = format!("{}.{}", call.device, call.method);
        let done_event = format!("{}.done", call.device);
        if write_output && self.wiring.observed(&call.output) {
            for changed in [true, false] {
                let id = format!(
                    "{base}.done.{}",
                    if changed { "changed" } else { "unchanged" }
                );
                let label = format!(
                    "{name} done · {} {}",
                    call.output,
                    if changed { "changed" } else { "unchanged" }
                );
                self.transition(&id, &label, Some(call.span), Vec::new());
                self.arc(from, &id, EdgeKind::Call);
                self.arc(&id, to, EdgeKind::Result);
                self.emit(&id, &done_event);
                if changed {
                    self.emit_change(&id, &call.output);
                }
            }
        } else {
            let id = format!("{base}.done");
            let label = if write_output {
                format!("{name} done · {} written", call.output)
            } else {
                format!("{name} done")
            };
            self.transition(&id, &label, Some(call.span), Vec::new());
            self.arc(from, &id, EdgeKind::Call);
            self.arc(&id, to, EdgeKind::Result);
            self.emit(&id, &done_event);
        }
        let failed = format!("{base}.failed");
        self.transition(
            &failed,
            &format!("{name} fails"),
            Some(call.span),
            Vec::new(),
        );
        self.arc(from, &failed, EdgeKind::Call);
        self.arc(&failed, to, EdgeKind::Result);
        self.emit(&failed, &format!("{}.failed", call.device));
        if let Some(limit) = call.timeout {
            // A timeout raises both `.timeout` and `.failed`.
            let expired = format!("{base}.timeout");
            self.transition(
                &expired,
                &format!("{name} times out"),
                Some(call.span),
                vec![format!("after {}", duration(limit.millis))],
            );
            self.arc(from, &expired, EdgeKind::Timeout)
                .security_significant = true;
            self.arc(&expired, to, EdgeKind::Result);
            self.emit(&expired, &format!("{}.timeout", call.device));
            self.emit(&expired, &format!("{}.failed", call.device));
        }
    }
}

#[derive(Default)]
struct Steps {
    process: usize,
    steps: usize,
    places: usize,
}
