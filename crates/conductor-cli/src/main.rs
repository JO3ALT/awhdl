use awhdl_checker::{Diagnostic, check};
use awhdl_parser::parse;
use clap::{Parser, Subcommand, ValueEnum};
use serde::Serialize;
use std::fs;
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
