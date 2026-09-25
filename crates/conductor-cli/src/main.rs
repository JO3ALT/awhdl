use awhdl_checker::{Diagnostic, check};
use awhdl_graph::{Format, Options, View, project, render};
use awhdl_parser::parse;
use clap::{Parser, Subcommand, ValueEnum};
use serde::Serialize;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Debug, Parser)]
#[command(name = "aic", version, about = "AI Conductor command-line interface")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Parse and statically check an AWHDL source file.
    Check {
        file: PathBuf,
        #[arg(long, value_enum, default_value_t = Output::Text)]
        output: Output,
    },
    /// Draw a checked AWHDL design as a diagram (Mermaid, Graphviz DOT or JSON).
    ///
    /// Exit codes: 0 ok, 1 parse error, 2 static error, 3 information-flow
    /// error, 4 projection error, 5 the format cannot express the view, 6 I/O
    /// error. An invalid design is never drawn.
    Graph {
        file: PathBuf,
        #[arg(long, value_enum, default_value_t = GraphView::Structure)]
        view: GraphView,
        #[arg(long, value_enum, default_value_t = GraphFormat::Mermaid)]
        format: GraphFormat,
        /// Output file; `-` writes to standard output.
        #[arg(long, default_value = "-")]
        output: PathBuf,
        /// Architecture to draw when the file has more than one.
        #[arg(long)]
        architecture: Option<String>,
        #[arg(long)]
        show_classification: bool,
        /// Show device generics such as `route`.
        #[arg(long)]
        show_capabilities: bool,
        /// Show budgets and assertions.
        #[arg(long)]
        show_policies: bool,
        /// Show device outcomes (.done/.failed/.timeout) that no process reacts to.
        #[arg(long)]
        show_internal: bool,
        /// One-line labels; security-significant detail is kept.
        #[arg(long)]
        compact: bool,
        /// Do not draw location lanes (the local/cloud boundary); JSON keeps them.
        #[arg(long)]
        no_lanes: bool,
        /// Check and project, but write nothing.
        #[arg(long)]
        validate_only: bool,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum GraphView {
    Structure,
    Behavior,
    State,
    Petri,
    Security,
    Activity,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum GraphFormat {
    Mermaid,
    Dot,
    Json,
    /// UML activity diagram (activity view only).
    Plantuml,
    /// PNML place/transition net (petri view only).
    Pnml,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Output {
    Text,
    Json,
}

#[derive(Debug, Serialize)]
struct JsonResult<'a> {
    ok: bool,
    file: &'a Path,
    diagnostics: Vec<RenderedDiagnostic>,
}

#[derive(Debug, Serialize)]
struct RenderedDiagnostic {
    code: String,
    message: String,
    line: usize,
    column: usize,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Check { file, output } => run_check(&file, output),
        Command::Graph {
            file,
            view,
            format,
            output,
            architecture,
            show_classification,
            show_capabilities,
            show_policies,
            show_internal,
            compact,
            no_lanes,
            validate_only,
        } => {
            let options = Options {
                architecture,
                show_classification,
                show_capabilities,
                show_policies,
                show_internal,
                compact,
            };
            let view = match view {
                GraphView::Structure => View::Structure,
                GraphView::Behavior => View::Behavior,
                GraphView::State => View::State,
                GraphView::Petri => View::Petri,
                GraphView::Security => View::Security,
                GraphView::Activity => View::Activity,
            };
            let format = match format {
                GraphFormat::Mermaid => Format::Mermaid,
                GraphFormat::Dot => Format::Dot,
                GraphFormat::Json => Format::Json,
                GraphFormat::Plantuml => Format::Plantuml,
                GraphFormat::Pnml => Format::Pnml,
            };
            run_graph(
                &file,
                view,
                format,
                &output,
                &options,
                no_lanes,
                validate_only,
            )
        }
    }
}

fn run_graph(
    path: &Path,
    view: View,
    format: Format,
    output: &Path,
    options: &Options,
    no_lanes: bool,
    validate_only: bool,
) -> ExitCode {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("{}: AWHDL-E001: {error}", path.display());
            return ExitCode::from(6);
        }
    };
    let design = match parse(&source) {
        Ok(design) => design,
        Err(error) => {
            eprintln!(
                "{}:{}:{}: AWHDL-E101: {}",
                path.display(),
                error.line,
                error.column,
                error.message
            );
            return ExitCode::from(1);
        }
    };
    let diagnostics = check(&design);
    if !diagnostics.is_empty() {
        let flow_only = diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code.starts_with("AWHDL-E3"));
        let rendered = diagnostics
            .into_iter()
            .map(|diagnostic| render_diagnostic(&source, diagnostic))
            .collect();
        emit(path, Output::Text, rendered);
        eprintln!(
            "{}: not drawn: the design does not pass aic check",
            path.display()
        );
        return ExitCode::from(if flow_only { 3 } else { 2 });
    }
    let mut graph = match project(&design, &source, view, options) {
        Ok(graph) => graph,
        Err(error) => {
            eprintln!("{}: AWHDL-G401: {error}", path.display());
            return ExitCode::from(4);
        }
    };
    if no_lanes && format != Format::Json {
        graph.lanes.clear();
    }
    let text = match render(&graph, format) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("{}: AWHDL-G501: {error}", path.display());
            return ExitCode::from(5);
        }
    };
    if validate_only {
        println!("OK {}", path.display());
        return ExitCode::SUCCESS;
    }
    let written = if output == Path::new("-") {
        std::io::stdout().write_all(text.as_bytes())
    } else {
        fs::write(output, text)
    };
    match written {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{}: AWHDL-E002: {error}", output.display());
            ExitCode::from(6)
        }
    }
}

fn run_check(path: &Path, output: Output) -> ExitCode {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("{}: AWHDL-E001: {error}", path.display());
            return ExitCode::from(1);
        }
    };

    let design = match parse(&source) {
        Ok(design) => design,
        Err(error) => {
            let diagnostic = RenderedDiagnostic {
                code: "AWHDL-E101".to_owned(),
                message: error.message,
                line: error.line,
                column: error.column,
            };
            emit(path, output, vec![diagnostic]);
            return ExitCode::from(2);
        }
    };

    let diagnostics = check(&design)
        .into_iter()
        .map(|diagnostic| render_diagnostic(&source, diagnostic))
        .collect::<Vec<_>>();
    let ok = diagnostics.is_empty();
    emit(path, output, diagnostics);
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(3)
    }
}

fn render_diagnostic(source: &str, diagnostic: Diagnostic) -> RenderedDiagnostic {
    let (line, column) = line_column(source, diagnostic.span.start);
    RenderedDiagnostic {
        code: diagnostic.code.to_owned(),
        message: diagnostic.message,
        line,
        column,
    }
}

fn emit(path: &Path, output: Output, diagnostics: Vec<RenderedDiagnostic>) {
    let ok = diagnostics.is_empty();
    match output {
        Output::Text if ok => println!("OK {}", path.display()),
        Output::Text => {
            for diagnostic in diagnostics {
                eprintln!(
                    "{}:{}:{}: {}: {}",
                    path.display(),
                    diagnostic.line,
                    diagnostic.column,
                    diagnostic.code,
                    diagnostic.message
                );
            }
        }
        Output::Json => println!(
            "{}",
            serde_json::to_string_pretty(&JsonResult {
                ok,
                file: path,
                diagnostics,
            })
            .expect("JSON serialization cannot fail")
        ),
    }
}

fn line_column(source: &str, offset: usize) -> (usize, usize) {
    let before = &source[..offset.min(source.len())];
    let line = before.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = before
        .rsplit_once('\n')
        .map_or(before.len() + 1, |(_, tail)| tail.len() + 1);
    (line, column)
}
