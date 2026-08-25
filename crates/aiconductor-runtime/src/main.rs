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
