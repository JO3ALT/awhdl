use aiconductor::config::ProjectConfig;
use aiconductor::engine::Engine;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(version, about = "AIConductor secure orchestration runtime")]
struct Cli {
    #[arg(long, default_value = ".")]
    project_root: PathBuf,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Check,
    Run {
        #[arg(long, default_value = "-")]
        prompt: String,
    },
    /// Compile and run an AWHDL design; device calls use configured routes.
    RunDesign {
        file: PathBuf,
        /// Input port value as name=value; the value is JSON when it parses as JSON.
        #[arg(long = "input")]
        inputs: Vec<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()))
        .with_writer(io::stderr)
        .init();
    let cli = Cli::parse();
    let config = ProjectConfig::load(&cli.project_root)?;
    match cli.command {
        Commands::Check => {
            println!("configuration ok: {}", config.root.display());
        }
        Commands::Run { prompt } => {
            let prompt = read_prompt(&prompt)?;
            let engine = Engine::new(config)?;
            println!("{}", engine.run(&prompt).await?);
        }
        Commands::RunDesign { file, inputs } => {
            let source = std::fs::read_to_string(&file)
                .with_context(|| format!("failed to read design: {}", file.display()))?;
            let mut values = std::collections::BTreeMap::new();
            for input in inputs {
                let (name, value) = input
                    .split_once('=')
                    .with_context(|| format!("input must be name=value: {input}"))?;
                let value = serde_json::from_str(value)
                    .unwrap_or_else(|_| serde_json::Value::String(value.to_owned()));
                values.insert(name.to_owned(), value);
            }
            let engine = Engine::new(config)?;
            let outcome = engine.run_design(&source, values).await?;
            println!("{}", serde_json::to_string_pretty(&outcome)?);
        }
    }
    Ok(())
}

fn read_prompt(value: &str) -> Result<String> {
    if value == "-" {
        let mut input = String::new();
        io::stdin().read_to_string(&mut input)?;
        Ok(input)
    } else {
        std::fs::read_to_string(Path::new(value))
            .with_context(|| format!("failed to read prompt file: {value}"))
    }
}
