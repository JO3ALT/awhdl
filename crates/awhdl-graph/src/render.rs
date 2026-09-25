//! Renderers: visualization IR -> Mermaid / Graphviz DOT / JSON / PlantUML
//! (activity view) / PNML (petri view).
//!
//! Renderers only present the IR. Every label is escaped, so text taken from
//! the source (expressions, string generics) can never become diagram syntax.
use crate::{Edge, Graph, Node, NodeKind, View};
use std::fmt::Write as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Mermaid,
    Dot,
    Json,
    /// UML activity diagram; activity view only.
    Plantuml,
    /// Petri Net Markup Language (ISO/IEC 15909-2) P/T net; petri view only.
    Pnml,
}

/// A format that cannot express the requested view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderError(pub String);

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RenderError {}

pub fn render(graph: &Graph, format: Format) -> Result<String, RenderError> {
    match format {
        Format::Mermaid => Ok(mermaid(graph)),
        Format::Dot => Ok(dot(graph)),
        Format::Json => {
            let mut text =
                serde_json::to_string_pretty(graph).expect("graph serialization cannot fail");
            text.push('\n');
            Ok(text)
        }
        Format::Plantuml => plantuml(graph),
        Format::Pnml => pnml(graph),
    }
}

fn view_name(view: View) -> &'static str {
    match view {
        View::Structure => "structure",
        View::Behavior => "behavior",
        View::State => "state",
        View::Petri => "petri",
        View::Security => "security",
        View::Activity => "activity",
    }
}

/// Display lines of a node: a Petri place shows its tokens first.
fn tokens(node: &Node) -> Option<String> {
    let count = node.metadata.get("tokens")?.parse::<usize>().ok()?;
    Some("●".repeat(count))
}

/// Unlabelled control nodes get a readable word where the shape needs text.
fn display_label(node: &Node) -> Vec<String> {
    let mut lines = node.label.clone();
    if lines.iter().all(|line| line.is_empty()) {
        lines = vec![
            match node.kind {
                NodeKind::Fork => "fork",
                NodeKind::Join => "join",
                _ => " ",
            }
            .to_owned(),
        ];
    }
    if let Some(tokens) = tokens(node) {
        lines.insert(0, tokens);
    }
    lines
}

fn title(graph: &Graph) -> String {
    let view = view_name(graph.view);
    format!(
        "AWHDL {view} view: entity {} / architecture {}",
        graph.entity, graph.architecture
    )
}

fn direction(view: View) -> &'static str {
    match view {
        View::Structure | View::Security => "LR",
        View::Behavior | View::State | View::Petri | View::Activity => "TD",
    }
}

/// Groups in declaration order, children after their parent.
fn child_groups<'g>(graph: &'g Graph, parent: Option<&str>) -> Vec<&'g crate::Group> {
    graph
        .groups
        .iter()
        .filter(|group| group.parent.as_deref() == parent)
        .collect()
}

fn members<'g>(graph: &'g Graph, group: Option<&str>) -> Vec<&'g Node> {
    let known = |id: &str| graph.groups.iter().any(|group| group.id == id);
    graph
        .nodes
        .iter()
        .filter(|node| match (&node.group, group) {
            (Some(own), Some(group)) => own == group,
            (Some(own), None) => !known(own),
            (None, group) => group.is_none(),
        })
        .collect()
}

/// Lanes are drawn when the view has at least two (the local/cloud boundary).
fn lanes_drawn(graph: &Graph) -> bool {
    graph.lanes.len() >= 2
}

fn in_lane(node: &Node, lane: Option<&str>) -> bool {
    lane.is_none_or(|lane| node.lane.as_deref() == Some(lane))
}

/// Whether a group, or any group nested in it, has a node in `lane`.
fn group_in_lane(graph: &Graph, group: &str, lane: Option<&str>) -> bool {
    graph
        .nodes
        .iter()
        .any(|node| node.group.as_deref() == Some(group) && in_lane(node, lane))
        || child_groups(graph, Some(group))
            .iter()
            .any(|child| group_in_lane(graph, &child.id, lane))
}

/// Fill and border of a lane; the lane label carries the meaning too.
fn lane_colors(lane: &str) -> (&'static str, &'static str) {
    match lane {
        "local" => ("#edf7ed", "#2e7d32"),
        "cloud" => ("#e8f0fb", "#1565c0"),
        _ => ("#f3f3f3", "#616161"),
    }
}

/// A lane label for renderers that print it raw: letters, digits, spaces,
/// parentheses and `_-.` only (lane labels come from location names).
fn lane_label(text: &str) -> String {
    text.chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || " ()_-.".contains(*ch))
        .collect()
}

/// A single comment line: control characters (newlines included) become spaces.
fn comment(text: &str) -> String {
    text.chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect()
}

/// One edge as drawn. `group` is set when the edge stands for the same denied
/// edge from every node of that group; the IR keeps the individual edges.
struct Drawn<'g> {
    edge: &'g Edge,
    group: Option<&'g crate::Group>,
    /// IDs of the IR edges this drawn edge stands for.
    covers: Vec<&'g str>,
}

/// Bundle denied edges: when every node directly in a group (two or more) has
/// a denied edge to the same target with the same kind and label, draw one
/// edge from the group. Nothing is lost, because the group has no other nodes.
fn drawn_edges(graph: &Graph) -> Vec<Drawn<'_>> {
    let mut bundled = std::collections::BTreeSet::new();
    let mut bundles = std::collections::BTreeMap::new();
    for group in &graph.groups {
        let nodes = graph
            .nodes
            .iter()
            .filter(|node| node.group.as_deref() == Some(group.id.as_str()))
            .map(|node| node.id.as_str())
            .collect::<Vec<_>>();
        if nodes.len() < 2 {
            continue;
        }
        let mut by_target: std::collections::BTreeMap<_, Vec<&Edge>> =
            std::collections::BTreeMap::new();
        for edge in &graph.edges {
            if edge.denied && nodes.contains(&edge.from.as_str()) {
                by_target
                    .entry((edge.to.as_str(), edge.kind as u8, edge.label.as_deref()))
                    .or_default()
                    .push(edge);
            }
        }
        for edges in by_target.into_values() {
            if edges.len() == nodes.len() {
                let first = edges[0].id.as_str();
                bundled.extend(edges.iter().map(|edge| edge.id.as_str()));
                bundles.insert(first, (group, edges));
            }
        }
    }
    let mut drawn = Vec::new();
    for edge in &graph.edges {
        if let Some((group, edges)) = bundles.get(edge.id.as_str()) {
            drawn.push(Drawn {
                edge,
                group: Some(group),
                covers: edges.iter().map(|edge| edge.id.as_str()).collect(),
            });
        } else if !bundled.contains(edge.id.as_str()) {
            drawn.push(Drawn {
                edge,
                group: None,
                covers: vec![edge.id.as_str()],
            });
        }
    }
    drawn
}

fn source_note(node: &Node) -> String {
    match &node.source_ref {
        Some(at) => format!("{} (line {}, column {})", node.id, at.line, at.column),
        None => node.id.clone(),
    }
}

// ---------------------------------------------------------------------------
// Mermaid

/// A Mermaid identifier derived injectively from the IR ID: letters and digits
/// stay, `_` doubles, and every other character becomes `_` plus a code.
fn mermaid_id(id: &str) -> String {
    let mut out = String::from("n_");
    for ch in id.chars() {
        match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' => out.push(ch),
            '_' => out.push_str("__"),
            ':' => out.push_str("_C"),
            '.' => out.push_str("_D"),
            other => {
                let _ = write!(out, "_X{:x}_", other as u32);
            }
        }
    }
    out
}

/// Text safe inside a quoted Mermaid label or an edge label: anything that
/// could be syntax becomes an entity code.
fn mermaid_text(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if ch.is_control() {
            out.push(' ');
        } else if ch.is_alphanumeric() || " .,:;_-+=/*!?'@·".contains(ch) {
            out.push(ch);
        } else {
            let _ = write!(out, "#{};", ch as u32);
        }
    }
    out
}

fn mermaid_label(node: &Node) -> String {
    // UML initial and final nodes are small black circles without text.
    if matches!(node.kind, NodeKind::Initial | NodeKind::Final) {
        return " ".to_owned();
    }
    display_label(node)
        .iter()
        .map(|line| mermaid_text(line))
        .collect::<Vec<_>>()
        .join("<br/>")
}

fn mermaid_shape(kind: NodeKind) -> (&'static str, &'static str) {
    match kind {
        NodeKind::Device => ("[[\"", "\"]]"),
        NodeKind::Approval => ("[/\"", "\"\\]"),
        NodeKind::Declassifier | NodeKind::Gateway => ("[\\\"", "\"/]"),
        NodeKind::Process | NodeKind::State => ("(\"", "\")"),
        NodeKind::Event => (">\"", "\"]"),
        NodeKind::Invocation | NodeKind::Place => ("([\"", "\"])"),
        NodeKind::Timer => ("((\"", "\"))"),
        NodeKind::Barrier | NodeKind::Fork => ("{{\"", "\"}}"),
        NodeKind::Policy => ("[/\"", "\"/]"),
        NodeKind::Decision => ("{\"", "\"}"),
        NodeKind::Entity | NodeKind::Value | NodeKind::Transition => ("[\"", "\"]"),
        NodeKind::Initial => ("((\"", "\"))"),
        NodeKind::Final => ("(((\"", "\")))"),
        NodeKind::Action => ("(\"", "\")"),
        NodeKind::AcceptEvent => ("[\\\"", "\"\\]"),
        NodeKind::SendSignal => (">\"", "\"]"),
        NodeKind::Merge => ("{\"", "\"}"),
        NodeKind::Join => ("{{\"", "\"}}"),
    }
}

fn mermaid_class(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Device | NodeKind::Approval | NodeKind::Declassifier | NodeKind::Gateway => {
            "device"
        }
        NodeKind::Value | NodeKind::Place => "value",
        NodeKind::Policy => "policy",
        NodeKind::Transition | NodeKind::Invocation | NodeKind::Action => "action",
        NodeKind::Initial | NodeKind::Final => "terminal",
        _ => "control",
    }
}

fn mermaid(graph: &Graph) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "---\ntitle: \"{}\"\n---", mermaid_text(&title(graph)));
    let _ = writeln!(out, "flowchart {}", direction(graph.view));
    for note in &graph.annotations {
        let _ = writeln!(out, "    %% {}", comment(note));
    }
    if lanes_drawn(graph) {
        for lane in &graph.lanes {
            let id = mermaid_id(&format!("lane:{}", lane.id));
            let _ = writeln!(out, "    subgraph {id}[\"{}\"]", mermaid_text(&lane.label));
            mermaid_group(graph, None, 2, &mut out, Some(&lane.id));
            out.push_str("    end\n");
        }
    } else {
        mermaid_group(graph, None, 1, &mut out, None);
    }

    let mut denied = Vec::new();
    let mut released = Vec::new();
    for (index, drawn) in drawn_edges(graph).iter().enumerate() {
        let edge = drawn.edge;
        let arrow = mermaid_arrow(edge);
        let from = match drawn.group {
            Some(group) => {
                let _ = writeln!(out, "    %% bundles {}", comment(&drawn.covers.join(", ")));
                mermaid_id(&format!("group:{}", group.id))
            }
            None => mermaid_id(&edge.from),
        };
        let label = edge
            .label
            .as_deref()
            .filter(|label| !label.is_empty())
            .map(|label| format!("|{}|", mermaid_text(label)))
            .unwrap_or_default();
        let _ = writeln!(out, "    {from} {arrow}{label} {}", mermaid_id(&edge.to));
        if edge.denied {
            denied.push(index.to_string());
        }
        if edge.kind == crate::EdgeKind::Declassification {
            released.push(index.to_string());
        }
    }

    out.push_str("    classDef device stroke-width:2px\n");
    out.push_str("    classDef value stroke-dasharray:0\n");
    out.push_str("    classDef policy stroke:#c62828,stroke-width:2px\n");
    out.push_str("    classDef action stroke-width:2px\n");
    out.push_str("    classDef control stroke-dasharray:0\n");
    out.push_str("    classDef terminal fill:#222,color:#fff\n");
    for class in ["device", "value", "policy", "action", "control", "terminal"] {
        let ids = graph
            .nodes
            .iter()
            .filter(|node| mermaid_class(node.kind) == class)
            .map(|node| mermaid_id(&node.id))
            .collect::<Vec<_>>();
        if !ids.is_empty() {
            let _ = writeln!(out, "    class {} {class}", ids.join(","));
        }
    }
    if !denied.is_empty() {
        let _ = writeln!(
            out,
            "    linkStyle {} stroke:#c62828,color:#c62828",
            denied.join(",")
        );
    }
    if !released.is_empty() {
        let _ = writeln!(
            out,
            "    linkStyle {} stroke:#6a1b9a,stroke-width:3px,color:#6a1b9a",
            released.join(",")
        );
    }
    let granted = graph
        .nodes
        .iter()
        .filter(|node| node.metadata.get("release").map(String::as_str) == Some("granted"))
        .map(|node| mermaid_id(&node.id))
        .collect::<Vec<_>>();
    if !granted.is_empty() {
        out.push_str("    classDef release fill:#f3e5f5,stroke:#6a1b9a,stroke-width:2px\n");
        let _ = writeln!(out, "    class {} release", granted.join(","));
    }
    if lanes_drawn(graph) {
        for lane in &graph.lanes {
            let (fill, border) = lane_colors(&lane.id);
            let _ = writeln!(
                out,
                "    style {} fill:{fill},stroke:{border},stroke-width:2px",
                mermaid_id(&format!("lane:{}", lane.id))
            );
        }
    }
    out
}

fn mermaid_group(
    graph: &Graph,
    group: Option<&str>,
    depth: usize,
    out: &mut String,
    lane: Option<&str>,
) {
    let indent = "    ".repeat(depth);
    for node in members(graph, group)
        .into_iter()
        .filter(|node| in_lane(node, lane))
    {
        // In the Petri view a completed call is a transition, drawn as one.
        let (open, close) = match (graph.view, node.kind) {
            (View::Petri, NodeKind::Invocation) => mermaid_shape(NodeKind::Transition),
            (_, kind) => mermaid_shape(kind),
        };
        let _ = writeln!(out, "{indent}%% {}", comment(&source_note(node)));
        let _ = writeln!(
            out,
            "{indent}{}{open}{}{close}",
            mermaid_id(&node.id),
            mermaid_label(node)
        );
    }
    for child in child_groups(graph, group) {
        if !group_in_lane(graph, &child.id, lane) {
            continue;
        }
        // A group split across lanes appears once per lane, with its own ID.
        let id = match lane {
            Some(lane) => format!("lane:{lane}/group:{}", child.id),
            None => format!("group:{}", child.id),
        };
        let _ = writeln!(
            out,
            "{indent}subgraph {}[\"{}\"]",
            mermaid_id(&id),
            mermaid_text(&child.label)
        );
        mermaid_group(graph, Some(&child.id), depth + 1, out, lane);
        let _ = writeln!(out, "{indent}end");
    }
}

/// Line style carries meaning without color: `-.-x` denied, `==>` guarded,
/// `-.->` asynchronous, `-->` synchronous.
fn mermaid_arrow(edge: &Edge) -> &'static str {
    if edge.denied {
        "-.-x"
    } else if edge.guarded {
        "==>"
    } else if edge.asynchronous {
        "-.->"
    } else {
        "-->"
    }
}

// ---------------------------------------------------------------------------
// Graphviz DOT

fn dot_string(text: &str) -> String {
    let mut out = String::from("\"");
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            ch if ch.is_control() => out.push(' '),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn dot_shape(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Device => "shape=component",
        NodeKind::Approval => "shape=house",
        NodeKind::Declassifier | NodeKind::Gateway => "shape=invhouse",
        NodeKind::Process | NodeKind::State => "shape=box, style=rounded",
        NodeKind::Value | NodeKind::Entity => "shape=box",
        NodeKind::Event => "shape=cds",
        NodeKind::Invocation => "shape=box, style=\"rounded,bold\"",
        NodeKind::Timer => "shape=box, style=diagonals",
        NodeKind::Barrier | NodeKind::Fork => "shape=hexagon",
        NodeKind::Policy => "shape=octagon, color=\"#c62828\"",
        NodeKind::Place => "shape=ellipse",
        NodeKind::Transition => "shape=box, style=filled, fillcolor=\"#eeeeee\"",
        NodeKind::Decision => "shape=diamond",
        NodeKind::Initial => {
            "shape=circle, style=filled, fillcolor=black, width=0.25, fixedsize=true"
        }
        NodeKind::Final => {
            "shape=doublecircle, style=filled, fillcolor=black, width=0.2, fixedsize=true"
        }
        NodeKind::Action => "shape=box, style=rounded",
        NodeKind::AcceptEvent => "shape=cds, orientation=180",
        NodeKind::SendSignal => "shape=cds",
        NodeKind::Merge => "shape=diamond, width=0.3, height=0.3, fixedsize=true",
        NodeKind::Join => "shape=hexagon",
    }
}

/// Shape and label attributes of a DOT node.
fn dot_node(node: &Node) -> String {
    let bar = "shape=box, style=filled, fillcolor=black, height=0.06, width=1.2, fixedsize=true";
    match node.kind {
        // A Petri place is a small circle with its tokens inside and its name beside.
        NodeKind::Place => format!(
            "shape=circle, fixedsize=true, width=0.4, label={}, xlabel={}",
            dot_string(&tokens(node).unwrap_or_default()),
            dot_string(&node.label.join("\n"))
        ),
        NodeKind::Initial | NodeKind::Final => format!("{}, label=\"\"", dot_shape(node.kind)),
        // UML fork and join bars (the behavior view's labelled forks stay hexagons).
        NodeKind::Fork | NodeKind::Join if node.label.iter().all(|line| line.is_empty()) => {
            format!("{bar}, label=\"\"")
        }
        NodeKind::Merge => format!("{}, label=\"\"", dot_shape(node.kind)),
        // A granted release stands out by fill as well as by its label.
        NodeKind::Transition | NodeKind::Action
            if node.metadata.get("release").map(String::as_str) == Some("granted") =>
        {
            format!(
                "shape=box, style=\"filled,bold\", fillcolor=\"#f3e5f5\", color=\"#6a1b9a\", label={}",
                dot_string(&node.label.join("\n"))
            )
        }
        kind => format!(
            "{}, label={}",
            dot_shape(kind),
            dot_string(&node.label.join("\n"))
        ),
    }
}

fn dot(graph: &Graph) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "digraph {} {{",
        dot_string(&format!("awhdl_{}", graph.architecture))
    );
    let _ = writeln!(
        out,
        "  graph [rankdir={}, compound=true, forcelabels=true, fontname=\"Helvetica\", labelloc=t, label={}];",
        direction(graph.view),
        dot_string(&title(graph))
    );
    if graph.view == View::Petri {
        // Place names sit beside their circles; give them room.
        out.push_str("  graph [nodesep=0.8, ranksep=0.5];\n");
    }
    out.push_str("  node [fontname=\"Helvetica\", fontsize=10];\n");
    out.push_str("  edge [fontname=\"Helvetica\", fontsize=9];\n");
    for note in &graph.annotations {
        let _ = writeln!(out, "  // {}", comment(note));
    }
    if lanes_drawn(graph) {
        for lane in &graph.lanes {
            let (fill, border) = lane_colors(&lane.id);
            let _ = writeln!(
                out,
                "  subgraph {} {{",
                dot_string(&format!("cluster_lane_{}", lane.id))
            );
            let _ = writeln!(
                out,
                "    label={}; style=\"filled,rounded\"; fillcolor=\"{fill}\"; color=\"{border}\"; penwidth=2; fontsize=14;",
                dot_string(&lane.label)
            );
            dot_group(graph, None, 2, &mut out, Some(&lane.id));
            out.push_str("  }\n");
        }
    } else {
        dot_group(graph, None, 1, &mut out, None);
    }
    for drawn in drawn_edges(graph) {
        let edge = drawn.edge;
        let mut attributes = Vec::new();
        if let Some(group) = drawn.group {
            // A compound edge leaves the cluster border (needs compound=true).
            attributes.push(format!(
                "ltail={}",
                dot_string(&format!("cluster_{}", group.id))
            ));
            attributes.push(format!(
                "tooltip={}",
                dot_string(&format!("bundles {}", drawn.covers.join(", ")))
            ));
        }
        if let Some(label) = edge.label.as_deref().filter(|label| !label.is_empty()) {
            attributes.push(format!("label={}", dot_string(label)));
        }
        if edge.denied {
            attributes.push(
                "style=dotted, color=\"#c62828\", fontcolor=\"#c62828\", arrowhead=tee".to_owned(),
            );
        } else if edge.kind == crate::EdgeKind::Declassification {
            attributes.push(
                "style=bold, penwidth=2.5, color=\"#6a1b9a\", fontcolor=\"#6a1b9a\"".to_owned(),
            );
        } else if edge.guarded {
            attributes.push("style=bold, penwidth=2".to_owned());
        } else if edge.asynchronous {
            attributes.push("style=dashed".to_owned());
        }
        if let Some(binding) = &edge.generation_binding {
            attributes.push(format!(
                "tooltip={}",
                dot_string(&format!("join key {binding}"))
            ));
        }
        attributes.push(format!("id={}", dot_string(&edge.id)));
        let _ = writeln!(
            out,
            "  {} -> {} [{}];",
            dot_string(&edge.from),
            dot_string(&edge.to),
            attributes.join(", ")
        );
    }
    out.push_str("}\n");
    out
}

fn dot_group(
    graph: &Graph,
    group: Option<&str>,
    depth: usize,
    out: &mut String,
    lane: Option<&str>,
) {
    let indent = "  ".repeat(depth);
    for node in members(graph, group)
        .into_iter()
        .filter(|node| in_lane(node, lane))
    {
        let _ = writeln!(
            out,
            "{indent}{} [{}, tooltip={}];",
            dot_string(&node.id),
            dot_node(node),
            dot_string(&source_note(node))
        );
    }
    for child in child_groups(graph, group) {
        if !group_in_lane(graph, &child.id, lane) {
            continue;
        }
        let cluster = match lane {
            Some(lane) => format!("cluster_{lane}_{}", child.id),
            None => format!("cluster_{}", child.id),
        };
        let _ = writeln!(out, "{indent}subgraph {} {{", dot_string(&cluster));
        let _ = writeln!(
            out,
            "{indent}  label={}; style=rounded;",
            dot_string(&child.label)
        );
        dot_group(graph, Some(&child.id), depth + 1, out, lane);
        let _ = writeln!(out, "{indent}}}");
    }
}

// ---------------------------------------------------------------------------
// PlantUML (activity view)

/// Text safe in a PlantUML activity label, condition or partition name:
/// anything that could end a shape or open markup becomes an HTML entity.
fn plantuml_text(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if ch.is_control() {
            out.push(' ');
        } else if ch.is_alphanumeric() || " .,:_-+=*!?'@·".contains(ch) {
            out.push(ch);
        } else {
            let _ = write!(out, "&#{};", ch as u32);
        }
    }
    out
}

fn plantuml(graph: &Graph) -> Result<String, RenderError> {
    if graph.view != View::Activity {
        return Err(RenderError(format!(
            "plantuml renders the activity view only, not the {} view",
            view_name(graph.view)
        )));
    }
    let mut out = String::from("@startuml\n");
    let _ = writeln!(out, "title {}", plantuml_text(&title(graph)));
    for note in &graph.annotations {
        let _ = writeln!(out, "' {}", comment(note));
    }
    if lanes_drawn(graph) {
        for lane in &graph.lanes {
            let _ = writeln!(
                out,
                "|{}|{}|",
                lane_colors(&lane.id).0,
                lane_label(&lane.label)
            );
        }
    }
    let walker = Walker::new(graph);
    for group in graph.groups.iter().filter(|group| group.kind == "activity") {
        let _ = writeln!(out, "partition \"{}\" {{", plantuml_text(&group.label));
        let initial = graph
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::Initial && node.group.as_deref() == Some(&group.id))
            .ok_or_else(|| RenderError(format!("activity {} has no initial node", group.id)))?;
        walker.walk(&initial.id, None, 1, &mut out)?;
        out.push_str("}\n");
    }
    out.push_str("@enduml\n");
    Ok(out)
}

/// Rebuilds the structured blocks of an activity from the IR: a decision
/// names its merge, a fork names its join.
struct Walker<'g> {
    nodes: std::collections::BTreeMap<&'g str, &'g Node>,
    outgoing: std::collections::BTreeMap<&'g str, Vec<&'g Edge>>,
    limit: usize,
    /// Lane label by lane id, when lanes are drawn.
    lanes: std::collections::BTreeMap<&'g str, String>,
    /// The lane the output is currently in.
    current: std::cell::RefCell<Option<String>>,
}

impl<'g> Walker<'g> {
    fn new(graph: &'g Graph) -> Self {
        let mut outgoing: std::collections::BTreeMap<&str, Vec<&Edge>> =
            std::collections::BTreeMap::new();
        for edge in &graph.edges {
            outgoing.entry(edge.from.as_str()).or_default().push(edge);
        }
        Walker {
            nodes: graph
                .nodes
                .iter()
                .map(|node| (node.id.as_str(), node))
                .collect(),
            outgoing,
            limit: graph.nodes.len() + graph.edges.len(),
            lanes: if lanes_drawn(graph) {
                graph
                    .lanes
                    .iter()
                    .map(|lane| (lane.id.as_str(), lane_label(&lane.label)))
                    .collect()
            } else {
                std::collections::BTreeMap::new()
            },
            current: std::cell::RefCell::new(None),
        }
    }

    fn next(&self, id: &str) -> Option<&'g str> {
        self.outgoing
            .get(id)
            .and_then(|edges| edges.first())
            .map(|edge| edge.to.as_str())
    }

    fn closer(&self, node: &Node, key: &str) -> Result<&'g str, RenderError> {
        let id = node
            .metadata
            .get(key)
            .ok_or_else(|| RenderError(format!("{} has no {key}", node.id)))?;
        self.nodes
            .get_key_value(id.as_str())
            .map(|(id, _)| *id)
            .ok_or_else(|| RenderError(format!("{} names unknown {key} {id}", node.id)))
    }

    fn walk(
        &self,
        start: &str,
        stop: Option<&str>,
        depth: usize,
        out: &mut String,
    ) -> Result<(), RenderError> {
        let indent = "  ".repeat(depth);
        let mut current = Some(start.to_owned());
        let mut steps = 0;
        while let Some(id) = current {
            steps += 1;
            if steps > self.limit {
                return Err(RenderError("activity is not structured".to_owned()));
            }
            if stop == Some(id.as_str()) {
                return Ok(());
            }
            let node = self.nodes[id.as_str()];
            let text = node
                .label
                .iter()
                .map(|line| plantuml_text(line))
                .collect::<Vec<_>>()
                .join("\\n");
            // Switch the swimlane before a node that lives in another lane.
            if let Some(label) = node.lane.as_deref().and_then(|lane| self.lanes.get(lane)) {
                let mut current = self.current.borrow_mut();
                if current.as_deref() != Some(label.as_str()) {
                    let _ = writeln!(out, "|{label}|");
                    *current = Some(label.clone());
                }
            }
            let mut resume = id.clone();
            match node.kind {
                NodeKind::Initial => {
                    let _ = writeln!(out, "{indent}start");
                }
                NodeKind::Final => {
                    let _ = writeln!(out, "{indent}stop");
                    return Ok(());
                }
                NodeKind::Action => {
                    let _ = writeln!(out, "{indent}:{text};");
                }
                NodeKind::AcceptEvent => {
                    let _ = writeln!(out, "{indent}:{text}; <<input>>");
                }
                NodeKind::SendSignal => {
                    let _ = writeln!(out, "{indent}:{text}; <<output>>");
                }
                NodeKind::Decision => {
                    let merge = self.closer(node, "merge")?;
                    let arms = self.outgoing.get(id.as_str()).cloned().unwrap_or_default();
                    let condition = |edge: &Edge| {
                        let raw = edge
                            .condition
                            .clone()
                            .or_else(|| edge.label.clone())
                            .unwrap_or_default();
                        let bare = raw
                            .strip_prefix('[')
                            .and_then(|rest| rest.strip_suffix(']'))
                            .map(str::to_owned)
                            .unwrap_or(raw);
                        plantuml_text(&bare)
                    };
                    if node.metadata.get("style").map(String::as_str) == Some("switch") {
                        let _ = writeln!(out, "{indent}switch ({text})");
                        // Empty cases last: PlantUML overlaps a leading empty case's label.
                        let mut arms = arms.clone();
                        arms.sort_by_key(|arm| arm.to == merge);
                        for arm in &arms {
                            let _ = writeln!(out, "{indent}case ({})", condition(arm));
                            if arm.to != merge {
                                self.walk(&arm.to, Some(merge), depth + 1, out)?;
                            }
                        }
                        let _ = writeln!(out, "{indent}endswitch");
                    } else {
                        // `if` statements carry their conditions on the arms; a
                        // question label (the process deadline) carries answers.
                        let question = node.label.first().map(String::as_str) != Some("if");
                        for (index, arm) in arms.iter().enumerate() {
                            let label = condition(arm);
                            let header = if index == 0 && question {
                                format!("if ({text}) then ({label})")
                            } else if index == 0 {
                                format!("if ({label}) then (yes)")
                            } else if question || (index + 1 == arms.len() && label == "else") {
                                format!("else ({label})")
                            } else {
                                format!("elseif ({label}) then (yes)")
                            };
                            let _ = writeln!(out, "{indent}{header}");
                            if arm.to != merge {
                                self.walk(&arm.to, Some(merge), depth + 1, out)?;
                            }
                        }
                        let _ = writeln!(out, "{indent}endif");
                    }
                    resume = merge.to_owned();
                }
                NodeKind::Fork => {
                    let join = self.closer(node, "join")?;
                    let _ = writeln!(out, "{indent}fork");
                    let arms = self.outgoing.get(id.as_str()).cloned().unwrap_or_default();
                    for (index, arm) in arms.iter().enumerate() {
                        if index > 0 {
                            let _ = writeln!(out, "{indent}fork again");
                        }
                        self.walk(&arm.to, Some(join), depth + 1, out)?;
                    }
                    let _ = writeln!(out, "{indent}end fork");
                    resume = join.to_owned();
                }
                NodeKind::Merge | NodeKind::Join => {}
                other => {
                    return Err(RenderError(format!(
                        "{other:?} node {} in an activity",
                        node.id
                    )));
                }
            }
            current = self.next(&resume).map(str::to_owned);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// PNML (petri view)

fn xml_text(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            ch if ch.is_control() => out.push(' '),
            ch => out.push(ch),
        }
    }
    out
}

/// A P/T net in PNML 2009. IDs are the Mermaid IDs, which are XML NCNames.
/// Arcs between the same place and transition are written once (weight 1).
fn pnml(graph: &Graph) -> Result<String, RenderError> {
    if graph.view != View::Petri {
        return Err(RenderError(format!(
            "pnml renders the petri view only, not the {} view",
            view_name(graph.view)
        )));
    }
    let kind = |id: &str| {
        graph
            .nodes
            .iter()
            .find(|node| node.id == id)
            .map(|node| node.kind)
    };
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<pnml xmlns=\"http://www.pnml.org/version-2009/grammar/pnml\">\n");
    let _ = writeln!(
        out,
        "  <net id=\"{}\" type=\"http://www.pnml.org/version-2009/grammar/ptnet\">",
        mermaid_id(&format!("net:{}", graph.architecture))
    );
    let _ = writeln!(
        out,
        "    <name><text>{}</text></name>",
        xml_text(&title(graph))
    );
    out.push_str("    <page id=\"page\">\n");
    for node in &graph.nodes {
        let name = xml_text(&node.label.join(" "));
        match node.kind {
            NodeKind::Place => {
                let _ = write!(
                    out,
                    "      <place id=\"{}\"><name><text>{name}</text></name>",
                    mermaid_id(&node.id)
                );
                if let Some(count) = node.metadata.get("tokens") {
                    let _ = write!(
                        out,
                        "<initialMarking><text>{}</text></initialMarking>",
                        xml_text(count)
                    );
                }
                out.push_str("</place>\n");
            }
            NodeKind::Transition => {
                let _ = writeln!(
                    out,
                    "      <transition id=\"{}\"><name><text>{name}</text></name></transition>",
                    mermaid_id(&node.id)
                );
            }
            other => {
                return Err(RenderError(format!(
                    "{other:?} node {} is not a place or transition",
                    node.id
                )));
            }
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    for edge in &graph.edges {
        let (from, to) = (kind(&edge.from), kind(&edge.to));
        if from == to {
            return Err(RenderError(format!(
                "arc {} does not join a place and a transition",
                edge.id
            )));
        }
        if !seen.insert((edge.from.as_str(), edge.to.as_str())) {
            continue;
        }
        let _ = writeln!(
            out,
            "      <arc id=\"a{}\" source=\"{}\" target=\"{}\"/>",
            seen.len(),
            mermaid_id(&edge.from),
            mermaid_id(&edge.to)
        );
    }
    out.push_str("    </page>\n  </net>\n</pnml>\n");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mermaid_ids_are_injective_and_safe() {
        assert_eq!(mermaid_id("event:q.done"), "n_event_Cq_Ddone");
        assert_ne!(mermaid_id("event:q.done"), mermaid_id("event:q_done"));
        assert_ne!(mermaid_id("value:a_.b"), mermaid_id("value:a._b"));
        assert!(
            mermaid_id("value:end")
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        );
    }

    #[test]
    fn denied_edges_from_a_whole_group_are_drawn_once() {
        let source = include_str!("../../../tests/graph/secure_development.awhdl");
        let design = awhdl_parser::parse(source).unwrap();
        let graph = crate::project(
            &design,
            source,
            crate::View::Security,
            &crate::Options::default(),
        )
        .unwrap();
        // Seven restricted values each keep their own denied edge in the IR ...
        let restricted = graph
            .edges
            .iter()
            .filter(|edge| edge.denied && edge.to == "policy:never.cloud")
            .filter(|edge| edge.classification.as_deref() == Some("restricted"))
            .count();
        assert_eq!(restricted, 7);
        // ... but each class group is drawn as one edge in both renderers.
        let mermaid = render(&graph, Format::Mermaid).unwrap();
        assert_eq!(
            mermaid
                .matches("n_group_Cclass_Crestricted -.-x|DENIED · restricted|")
                .count(),
            1
        );
        assert!(!mermaid.contains("n_value_Csource -.-x"));
        let dot = render(&graph, Format::Dot).unwrap();
        assert_eq!(dot.matches("ltail=\"cluster_class:restricted\"").count(), 1);
        assert_eq!(dot.matches("ltail=\"cluster_class:internal\"").count(), 1);
        // A group that is not wholly denied (public) is never bundled.
        assert!(!dot.contains("cluster_class:public\", tooltip"));
    }

    fn secure(view: crate::View) -> Graph {
        let source = include_str!("../../../tests/graph/secure_development.awhdl");
        let design = awhdl_parser::parse(source).unwrap();
        crate::project(&design, source, view, &crate::Options::default()).unwrap()
    }

    #[test]
    fn plantuml_and_pnml_render_only_the_view_they_can_express() {
        let petri = secure(crate::View::Petri);
        let activity = secure(crate::View::Activity);
        assert!(render(&petri, Format::Plantuml).is_err());
        assert!(render(&activity, Format::Pnml).is_err());
        assert!(render(&secure(crate::View::Security), Format::Pnml).is_err());

        let uml = render(&activity, Format::Plantuml).unwrap();
        assert!(uml.starts_with("@startuml\n") && uml.ends_with("@enduml\n"));
        assert_eq!(uml.matches("partition ").count(), 7);
        assert_eq!(uml.matches("\n  start\n").count(), 7);
        for block in [
            "switch (local_coder.run)",
            "fork again",
            "end fork",
            "elseif (",
            "; <<input>>",
            "; <<output>>",
        ] {
            assert!(uml.contains(block), "{block} missing");
        }
        let lines = |prefix: &str| {
            uml.lines()
                .filter(|line| line.trim_start().starts_with(prefix))
                .count()
        };
        assert_eq!(lines("if ("), lines("endif"));
        assert_eq!(lines("switch ("), lines("endswitch"));
        let exact = |text: &str| uml.lines().filter(|line| line.trim() == text).count();
        assert_eq!(exact("fork"), exact("end fork"));

        let net = render(&petri, Format::Pnml).unwrap();
        assert!(net.contains("type=\"http://www.pnml.org/version-2009/grammar/ptnet\""));
        assert_eq!(net.matches("<initialMarking>").count(), 2);
        let places = petri
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Place)
            .count();
        assert_eq!(net.matches("<place ").count(), places);
        assert_eq!(
            net.matches("<transition ").count(),
            petri.nodes.len() - places
        );
    }

    #[test]
    fn lanes_are_drawn_only_when_there_is_a_boundary() {
        let mut activity = secure(crate::View::Activity);
        let uml = render(&activity, Format::Plantuml).unwrap();
        assert!(uml.contains("|#edf7ed|local (protected)|\n|#e8f0fb|cloud (external)|\n"));
        // The cloud call switches lanes and the flow switches back.
        let cloud = uml.find("\n|cloud (external)|\n").expect("switch to cloud");
        assert!(uml[cloud..].contains(":review &#60;= cloud_reviewer.review&#40;note&#41;;"));
        assert!(uml[cloud..].contains("\n|local (protected)|\n"));
        let dot = render(&activity, Format::Dot).unwrap();
        assert!(dot.contains("cluster_lane_local") && dot.contains("cluster_lane_cloud"));
        assert!(dot.contains("\"cluster_cloud_activity:p4\""));
        let mermaid = render(&activity, Format::Mermaid).unwrap();
        assert!(mermaid.contains("subgraph n_lane_Ccloud[\"cloud #40;external#41;\"]"));
        // --no-lanes clears them before rendering.
        activity.lanes.clear();
        assert!(
            !render(&activity, Format::Dot)
                .unwrap()
                .contains("cluster_lane_")
        );
        assert!(
            !render(&activity, Format::Plantuml)
                .unwrap()
                .contains("|local")
        );
    }

    #[test]
    fn plantuml_labels_cannot_inject_syntax() {
        let text = plantuml_text("x; <<output>>\nstop\n:evil| (a) [b] {c} /d\\");
        // Only entity references remain, and their `;` never ends a line.
        let mut rest = text.as_str();
        let mut plain = String::new();
        while let Some(start) = rest.find("&#") {
            plain.push_str(&rest[..start]);
            let end = rest[start..].find(';').expect("entity is closed") + start;
            assert!(rest[start + 2..end].chars().all(|ch| ch.is_ascii_digit()));
            rest = &rest[end + 1..];
        }
        plain.push_str(rest);
        let text = plain;
        for forbidden in [
            ';', '<', '>', '\n', '|', '(', ')', '[', ']', '{', '}', '/', '\\',
        ] {
            assert!(!text.contains(forbidden), "{forbidden:?} in {text}");
        }
        assert_eq!(xml_text("<a & 'b'>"), "&lt;a &amp; &apos;b&apos;&gt;");
    }

    #[test]
    fn labels_cannot_inject_syntax() {
        let hostile = "x\"] --> evil[\"pwn\n%% |click|";
        let text = mermaid_text(hostile);
        for forbidden in ['"', '[', ']', '|', '\n', '%', '<', '>'] {
            assert!(!text.contains(forbidden), "{forbidden:?} in {text}");
        }
        assert_eq!(dot_string("a\"b\\c\nd"), "\"a\\\"b\\\\c\\nd\"");
    }
}
