use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use futures::StreamExt;

use tokio::fs::read_to_string;
use tokio::io::{self, stdin, stdout, AsyncReadExt, AsyncWriteExt};

mod agents;
use agents::{AgentEvent, AgentRequester, OpenAIAgent};

#[derive(Parser, Debug)]
struct Args {
    #[arg(short, long)]
    agent: String,

    #[arg(short, long)]
    model: Option<String>,

    #[command(subcommand)]
    read_source: ReadSource,
}

#[derive(Subcommand, Default, Debug)]
enum ReadSource {
    #[default]
    Stdin,
    File {
        path: PathBuf,
    },
}

impl ReadSource {
    async fn read(&self) -> io::Result<String> {
        match self {
            Self::Stdin => {
                let mut buffer = String::new();
                stdin().read_to_string(&mut buffer).await.map(|_| buffer)
            }
            Self::File { path } => read_to_string(path).await,
        }
    }
}

async fn execute(read_source: ReadSource) -> Result<()> {
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
    execute(cli.read_source).await
}
