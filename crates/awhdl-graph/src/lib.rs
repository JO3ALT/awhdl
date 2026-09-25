//! Human-readable views of a checked AWHDL design.
//!
//! A view is a derived projection of the checked AST into a small
//! visualization IR ([`Graph`]); renderers turn that IR into Mermaid, Graphviz
//! DOT or JSON. Neither step changes execution semantics: the graph describes
//! what a design may do, not the normative runtime model.
use awhdl_ast::Design;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt;

mod project;
mod render;

pub use render::{Format, RenderError, render};

pub const GRAPH_VERSION: &str = "0.1";
pub const SOURCE_IR_VERSION: &str = "0.1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum View {
    Structure,
    Behavior,
    State,
    Petri,
    Security,
    Activity,
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Architecture to project; required when the file has more than one.
    pub architecture: Option<String>,
    pub show_classification: bool,
    pub show_capabilities: bool,
    pub show_policies: bool,
    /// Also show device outcomes (`.done` / `.failed` / `.timeout`) that no
    /// process is sensitive to.
    pub show_internal: bool,
    /// One-line labels; security-significant labels are kept.
    pub compact: bool,
}

/// A projection that would have to invent structure is refused, not drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionError(pub String);

impl fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ProjectionError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Graph {
    pub graph_version: &'static str,
    pub source_ir_version: &'static str,
    pub view: View,
    pub entity: String,
    pub architecture: String,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub groups: Vec<Group>,
    pub annotations: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Entity,
    Device,
    Process,
    Value,
    Event,
    Invocation,
    Timer,
    Barrier,
    State,
    Policy,
    Gateway,
    Approval,
    Declassifier,
    Place,
    Transition,
    /// An `if` statement (extension of the view-model node kinds).
    Decision,
    /// A `parallel` block (extension of the view-model node kinds); the fork
    /// bar in the activity view.
    Fork,
    /// Activity view (UML): initial node.
    Initial,
    /// Activity view (UML): activity final node.
    Final,
    /// Activity view (UML): action.
    Action,
    /// Activity view (UML): accept event action (signal or time event).
    AcceptEvent,
    /// Activity view (UML): send signal action.
    SendSignal,
    /// Activity view (UML): merge node closing a decision.
    Merge,
    /// Activity view (UML): join bar closing a fork.
    Join,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceRef {
    pub line: usize,
    pub column: usize,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Node {
    /// Stable identity derived from declarations and source order, never from labels.
    pub id: String,
    pub kind: NodeKind,
    /// Display lines; the first line is the name.
    pub label: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<SourceRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_kind: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub capabilities: BTreeMap<String, String>,
    pub generation_sensitive: bool,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    DataFlow,
    EventTrigger,
    Call,
    Result,
    StateTransition,
    SecurityFlow,
    ApprovalGate,
    Declassification,
    Fork,
    Join,
    Retry,
    Timeout,
    /// Sequential control inside a process (extension of the view-model edge kinds).
    Control,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Edge {
    pub id: String,
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    #[serde(rename = "async")]
    pub asynchronous: bool,
    pub guarded: bool,
    pub denied: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation_binding: Option<String>,
    /// Kept by `--compact` and never dropped by any renderer.
    pub security_significant: bool,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Group {
    pub id: String,
    /// `entity`, `zone`, `orchestrator` or `class`.
    pub kind: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

/// Project one architecture of a design that has already passed
/// `awhdl_checker::check`. `source` is used only for line/column references.
pub fn project(
    design: &Design,
    source: &str,
    view: View,
    options: &Options,
) -> Result<Graph, ProjectionError> {
    project::project(design, source, view, options)
}
