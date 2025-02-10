use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use futures::StreamExt;

use tokio::fs::read_to_string;
use tokio::io::{self, stdin, stdout, AsyncReadExt, AsyncWriteExt};

mod agents;
mod config;

use crate::agents::{AgentEvent, AgentRequester, OpenAIAgent};
use crate::config::Config;

#[derive(Parser, Debug)]
struct Args {
    #[arg(short, long)]
    agent: Option<String>,

    #[arg(short, long)]
    model: Option<String>,

    #[arg(short, long)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    read_source: Option<ReadSource>,
}

#[derive(Subcommand, Debug)]
enum ReadSource {
    Stdin,
    File { path: PathBuf },
}

impl ReadSource {
    async fn read(&self) -> io::Result<String> {
        match self {
            Self::Stdin => {
                let mut buffer = String::new();
                stdin().read_to_string(&mut buffer).await?;
                Ok(buffer)
            }
            Self::File { path } => read_to_string(path).await,
        }
    }
}

async fn execute(config: Config, read_source: ReadSource) -> Result<()> {
    let mut stdout = stdout();

    // TODO: improve error printing
    let output = read_source.read().await?;
    let mut eventsource = OpenAIAgent.request(&output)?;

    while let Some(event) = eventsource.next().await {
        match event {
            Ok(event) => match event {
                AgentEvent::Open => {}
                AgentEvent::Text(content) => {
                    let _ = stdout.write(content.as_bytes()).await;
                    let _ = stdout.flush().await;
                }
                AgentEvent::End => break,
            },
            Err(_) => todo!(),
        };
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Args::parse();
    let config = match cli.config {
        Some(p) => Config::from_path(p),
        None => {
            let config_path = config::default_config_path()?;
            if config_path.exists() {
                Config::from_path(config_path)
            } else {
                Ok(Config::default())
            }
        }
    }?;

    execute(config, cli.read_source.unwrap_or(ReadSource::Stdin)).await
}
