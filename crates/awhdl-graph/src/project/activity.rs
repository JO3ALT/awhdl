//! Activity view: a UML activity diagram with one activity per process.
//!
//! An activity starts by accepting the events of its sensitivity list, runs
//! its statements as actions, and sends a signal for every event it raises
//! that some process or barrier observes. Activities are connected by signal
//! names, as in UML, not by edges. Each barrier is an activity of its own that
//! joins its members and sends `barrier.ready`.
use super::petri::event_key;
use super::{Builder, Symbols, Trigger, clip, duration, trigger};
use crate::{EdgeKind, NodeKind};
use awhdl_ast::{DeviceCall, ProcessStmt, SequentialStatement, Span};
use std::collections::BTreeSet;

pub(super) const NOTE: &str = "UML activity diagram: one activity per process and per barrier, \
     connected by signal names. A value's signal is sent only when the write changes the value; \
     signals take effect when the process run commits.";

/// A region of an activity with one entry and one exit node.
type Region = (String, String);

pub(super) fn activity(b: &mut Builder, s: &Symbols) {
    b.annotate(NOTE);
    let observed = observed_events(s);
    for (index, process) in s.processes.iter().enumerate() {
        let mut act = Act {
            b: &mut *b,
            s,
            observed: &observed,
            scope: format!("p{}", index + 1),
            steps: 0,
        };
        act.process(process);
    }
    for barrier in &s.barriers {
        let mut act = Act {
            b: &mut *b,
            s,
            observed: &observed,
            scope: format!("barrier.{}", barrier.name),
            steps: 0,
        };
        act.barrier(barrier);
    }
}

/// Events some process waits for, plus changes of barrier members.
fn observed_events(s: &Symbols) -> BTreeSet<String> {
    let mut observed = s
        .processes
        .iter()
        .flat_map(|process| &process.sensitivity)
        .map(|text| event_key(s, text))
        .collect::<BTreeSet<_>>();
    for barrier in &s.barriers {
        observed.extend(barrier.members.iter().cloned());
    }
    observed
}

struct Act<'b, 'a, 's> {
    b: &'b mut Builder<'a>,
    s: &'b Symbols<'s>,
    observed: &'b BTreeSet<String>,
    /// `p<n>` or `barrier.<name>`; node IDs are `activity:<scope>.<suffix>`.
    scope: String,
    steps: usize,
}

impl Act<'_, '_, '_> {
    fn group(&self) -> String {
        format!("activity:{}", self.scope)
    }

    fn node(
        &mut self,
        suffix: &str,
        kind: NodeKind,
        label: &str,
        span: Option<Span>,
        significant: Vec<String>,
    ) -> String {
        let id = format!("activity:{}.{suffix}", self.scope);
        let group = self.group();
        self.b
            .node(&id, kind, label, span, Vec::new(), significant)
            .group = Some(group);
        id
    }

    fn flow(&mut self, from: &str, to: &str, condition: Option<String>) {
        let edge = self.b.edge(from, to, EdgeKind::Control, condition.clone());
        edge.condition = condition;
    }

    /// Link regions in order; `None` when there are none.
    fn sequence(&mut self, regions: Vec<Region>) -> Option<Region> {
        let mut iter = regions.into_iter();
        let (entry, mut exit) = iter.next()?;
        for (next_entry, next_exit) in iter {
            self.flow(&exit, &next_entry, None);
            exit = next_exit;
        }
        Some((entry, exit))
    }

    fn observes(&self, event: &str) -> bool {
        self.observed.contains(event) || self.b.options.show_internal
    }

    fn send(&mut self, suffix: &str, signal: &str) -> Region {
        let id = self.node(suffix, NodeKind::SendSignal, signal, None, Vec::new());
        (id.clone(), id)
    }

    fn process(&mut self, process: &ProcessStmt) {
        self.b.group(
            &self.group(),
            "activity",
            &format!("process({})", process.sensitivity.join(", ")),
            None,
        );
        let initial = self.node(
            "initial",
            NodeKind::Initial,
            "start",
            Some(process.span),
            Vec::new(),
        );
        let accepted = process
            .sensitivity
            .iter()
            .map(|text| match trigger(self.s, text) {
                Trigger::Value(name) => format!("{name} changed"),
                Trigger::Timer(timer) => {
                    format!("{} (every {})", timer.name, duration(timer.period.millis))
                }
                Trigger::Barrier(barrier) => format!("{}.ready", barrier.name),
                Trigger::Event(event) => event.to_owned(),
            })
            .collect::<Vec<_>>()
            .join(" | ");
        let accept = self.node(
            "accept",
            NodeKind::AcceptEvent,
            &accepted,
            Some(process.span),
            Vec::new(),
        );
        self.flow(&initial, &accept, None);
        let body = match process.timeout {
            None => self.statements(&process.statements),
            Some(limit) => {
                // The body either finishes in time or is discarded for `on timeout`.
                let decision = self.node(
                    "deadline",
                    NodeKind::Decision,
                    &format!("finished within {}?", duration(limit.millis)),
                    Some(process.span),
                    Vec::new(),
                );
                let merge = self.node("deadline.merge", NodeKind::Merge, "", None, Vec::new());
                self.pair(&decision, "merge", &merge, "if");
                let arms = [
                    ("[yes]".to_owned(), self.statements(&process.statements)),
                    ("[timeout]".to_owned(), self.statements(&process.on_timeout)),
                ];
                self.branches(&decision, &merge, arms.into_iter().collect());
                Some((decision, merge))
            }
        };
        let last = match body {
            Some((entry, exit)) => {
                self.flow(&accept, &entry, None);
                exit
            }
            None => accept,
        };
        let final_node = self.node("final", NodeKind::Final, "end", None, Vec::new());
        self.flow(&last, &final_node, None);
    }

    fn barrier(&mut self, barrier: &awhdl_ast::BarrierDecl) {
        self.b.group(
            &self.group(),
            "activity",
            &format!("barrier {} ({})", barrier.name, barrier.members.join(", ")),
            None,
        );
        let initial = self.node(
            "initial",
            NodeKind::Initial,
            "start",
            Some(barrier.span),
            Vec::new(),
        );
        let fork = self.node("fork", NodeKind::Fork, "", Some(barrier.span), Vec::new());
        let join = self.node("join", NodeKind::Join, "", Some(barrier.span), Vec::new());
        self.pair(&fork, "join", &join, "fork");
        self.b.nodes[self.b.node_index[&join]].generation_sensitive = true;
        self.flow(&initial, &fork, None);
        for member in &barrier.members {
            let accept = self.node(
                &format!("accept.{member}"),
                NodeKind::AcceptEvent,
                &format!("{member} changed"),
                None,
                Vec::new(),
            );
            self.flow(&fork, &accept, None);
            self.flow(&accept, &join, None);
            self.b
                .edge(&accept, &join, EdgeKind::Control, None)
                .generation_binding = Some(super::JOIN_KEY.to_owned());
        }
        let (ready, _) = self.send("ready", &format!("{}.ready", barrier.name));
        self.flow(&join, &ready, None);
        let final_node = self.node("final", NodeKind::Final, "end", None, Vec::new());
        self.flow(&ready, &final_node, None);
    }

    /// Record the node that closes a decision or fork, for structured renderers.
    fn pair(&mut self, open: &str, key: &str, close: &str, style: &str) {
        let node = &mut self.b.nodes[self.b.node_index[open]];
        node.metadata.insert(key.to_owned(), close.to_owned());
        node.metadata.insert("style".to_owned(), style.to_owned());
    }

    /// Branch edges from a decision to its merge, in order.
    fn branches(&mut self, decision: &str, merge: &str, arms: Vec<(String, Option<Region>)>) {
        for (condition, region) in arms {
            match region {
                Some((entry, exit)) => {
                    self.flow(decision, &entry, Some(condition));
                    self.flow(&exit, merge, None);
                }
                None => self.flow(decision, merge, Some(condition)),
            }
        }
    }

    fn statements(&mut self, statements: &[SequentialStatement]) -> Option<Region> {
        let regions = statements
            .iter()
            .filter_map(|statement| self.statement(statement))
            .collect();
        self.sequence(regions)
    }

    fn statement(&mut self, statement: &SequentialStatement) -> Option<Region> {
        self.steps += 1;
        let step = format!("s{}", self.steps);
        match statement {
            SequentialStatement::Assignment(assignment) => {
                let action = self.node(
                    &step,
                    NodeKind::Action,
                    &format!(
                        "{} <= {}",
                        assignment.target,
                        clip(&assignment.value.source)
                    ),
                    Some(assignment.span),
                    Vec::new(),
                );
                let mut regions = vec![(action.clone(), action)];
                if self.observes(&assignment.target) {
                    let signal = format!("{} changed", assignment.target);
                    regions.push(self.send(&format!("{step}.send"), &signal));
                }
                self.sequence(regions)
            }
            SequentialStatement::DeviceCall(call) => Some(self.call(&step, call, true)),
            SequentialStatement::If(branching) => {
                let decision = self.node(
                    &format!("{step}.if"),
                    NodeKind::Decision,
                    "if",
                    Some(branching.span),
                    Vec::new(),
                );
                let merge = self.node(
                    &format!("{step}.merge"),
                    NodeKind::Merge,
                    "",
                    None,
                    Vec::new(),
                );
                self.pair(&decision, "merge", &merge, "if");
                let mut arms = Vec::new();
                for branch in &branching.branches {
                    let region = self.statements(&branch.statements);
                    arms.push((format!("[{}]", clip(&branch.condition.source)), region));
                }
                let otherwise = self.statements(&branching.otherwise);
                arms.push(("[else]".to_owned(), otherwise));
                self.branches(&decision, &merge, arms);
                Some((decision, merge))
            }
            SequentialStatement::Parallel(parallel) => {
                let fork = self.node(
                    &format!("{step}.fork"),
                    NodeKind::Fork,
                    "",
                    Some(parallel.span),
                    Vec::new(),
                );
                let join = self.node(
                    &format!("{step}.join"),
                    NodeKind::Join,
                    "",
                    Some(parallel.span),
                    Vec::new(),
                );
                self.pair(&fork, "join", &join, "fork");
                self.b.nodes[self.b.node_index[&join]].generation_sensitive = true;
                for (j, call) in parallel.calls.iter().enumerate() {
                    let (entry, exit) = self.call(&format!("{step}.call{}", j + 1), call, false);
                    self.flow(&fork, &entry, None);
                    self.flow(&exit, &join, None);
                    self.b
                        .edge(&exit, &join, EdgeKind::Control, None)
                        .generation_binding = Some(super::JOIN_KEY.to_owned());
                }
                // Results commit together after the join.
                let outputs = parallel
                    .calls
                    .iter()
                    .map(|call| call.output.as_str())
                    .collect::<Vec<_>>();
                let commit = self.node(
                    &format!("{step}.commit"),
                    NodeKind::Action,
                    &format!("commit {}", outputs.join(", ")),
                    Some(parallel.span),
                    Vec::new(),
                );
                let mut regions = vec![(fork, join.clone()), (commit.clone(), commit)];
                for (j, output) in outputs.iter().enumerate() {
                    if self.observes(output) {
                        let signal = format!("{output} changed");
                        regions.push(self.send(&format!("{step}.send{}", j + 1), &signal));
                    }
                }
                self.sequence(regions)
            }
            SequentialStatement::Assert { condition, span } => {
                if !self.b.options.show_policies {
                    return None;
                }
                let action = self.node(
                    &step,
                    NodeKind::Action,
                    &format!("assert {}", clip(&condition.source)),
                    Some(*span),
                    Vec::new(),
                );
                Some((action.clone(), action))
            }
            SequentialStatement::Null { .. } => None,
        }
    }

    /// A call action, then a switch on its outcome when an outcome event is
    /// observed. A sequential call writes its output on success.
    fn call(&mut self, step: &str, call: &DeviceCall, write_output: bool) -> Region {
        let name = format!("{}.{}", call.device, call.method);
        let arguments = call
            .arguments
            .iter()
            .map(|argument| clip(&argument.source))
            .collect::<Vec<_>>()
            .join(", ");
        let label = if write_output {
            format!("{} <= {name}({arguments})", call.output)
        } else {
            format!("{name}({arguments})")
        };
        let significant = call
            .timeout
            .map(|limit| vec![format!("timeout {}", duration(limit.millis))])
            .unwrap_or_default();
        let action = self.node(step, NodeKind::Action, &label, Some(call.span), significant);
        let changed = write_output && self.observes(&call.output);
        let device = &call.device;
        let done = self.observes(&format!("{device}.done"));
        let failed = self.observes(&format!("{device}.failed"));
        let timeout = call.timeout.is_some() && self.observes(&format!("{device}.timeout"));
        if !(done || failed || timeout) {
            if !changed {
                return (action.clone(), action);
            }
            let send = self.send(&format!("{step}.send"), &format!("{} changed", call.output));
            return self
                .sequence(vec![(action.clone(), action), send])
                .expect("two regions");
        }
        let decision = self.node(
            &format!("{step}.outcome"),
            NodeKind::Decision,
            &name,
            Some(call.span),
            Vec::new(),
        );
        let merge = self.node(
            &format!("{step}.outcome.merge"),
            NodeKind::Merge,
            "",
            None,
            Vec::new(),
        );
        self.pair(&decision, "merge", &merge, "switch");
        self.flow(&action, &decision, None);
        let mut on_done = Vec::new();
        if changed {
            on_done.push(self.send(
                &format!("{step}.changed"),
                &format!("{} changed", call.output),
            ));
        }
        if done {
            on_done.push(self.send(&format!("{step}.done"), &format!("{device}.done")));
        }
        let mut arms = vec![("done".to_owned(), self.sequence(on_done))];
        let on_failed =
            failed.then(|| self.send(&format!("{step}.failed"), &format!("{device}.failed")));
        arms.push(("failed".to_owned(), on_failed));
        if call.timeout.is_some() {
            // A timeout raises both `.timeout` and `.failed`.
            let mut on_timeout = Vec::new();
            if timeout {
                on_timeout
                    .push(self.send(&format!("{step}.timeout"), &format!("{device}.timeout")));
            }
            if failed {
                on_timeout.push(self.send(
                    &format!("{step}.timeout.failed"),
                    &format!("{device}.failed"),
                ));
            }
            arms.push(("timeout".to_owned(), self.sequence(on_timeout)));
        }
        self.branches(&decision, &merge, arms);
        (action, merge)
    }
}
