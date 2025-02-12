use std::io::stdout;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

use anyhow::Result;
use clap::{Parser, Subcommand};
use futures::StreamExt;
use ratatui::{prelude::*, TerminalOptions, Viewport};

use tokio::fs::File;
use tokio::io::{self, stdin, AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::time::timeout;

mod agents;
mod config;
mod widgets;

use crate::agents::{AgentEvent, AgentRequester, OpenAIAgent};
use crate::config::Config;
use crate::widgets::{LoadingLine, LoadingLineState};

const MAX_INLINE_VIEW: usize = 7;

#[derive(Parser, Debug)]
struct Args {
    #[arg(short, long)]
    agent: Option<String>,

    #[arg(short, long)]
    model: Option<String>,

    #[arg(long = "no-line-number", default_value_t = false)]
    no_line_number: bool,

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
    async fn reader(&self) -> io::Result<BufReader<Pin<Box<dyn AsyncRead>>>> {
        let reader: Pin<Box<dyn AsyncRead>> = match self {
            Self::Stdin => Box::pin(stdin()),
            Self::File { path } => Box::pin(File::open(path).await?),
        };
        Ok(BufReader::new(reader))
    }
}

async fn get_source_data(
    terminal: &mut Terminal<impl Backend>,
    read_source: ReadSource,
    show_lines: bool,
) -> Result<Vec<String>> {
    let reader = read_source.reader().await?;
    let mut lines = reader.lines();
    let mut buffer = Vec::new();
    let mut lnum = 0usize;

    loop {
        if let Ok(line_res) = timeout(Duration::from_millis(300), lines.next_line()).await {
            if let Some(line) = line_res? {
                lnum += 1;

                if lnum == 200 {
                    terminal.insert_before(1, |buf| {
                        Line::from(" <... truncated output ...>")
                            .style(Style::default().italic().dark_gray())
                            .render(buf.area, buf);
                    })?;
                } else if lnum <= 199 {
                    terminal.insert_before(1, |buf| {
                        let spans: &[Span] = if show_lines {
                            &[
                                Span::from(format!(" {:4o} \u{2502} ", lnum)),
                                Span::from(line.as_str()),
                            ]
                        } else {
                            &[Span::from(line.as_str())]
                        };
                        Line::from(spans.to_vec())
                            .style(Style::default().dark_gray())
                            .render(buf.area, buf);
                    })?;
                }
                buffer.push(line);
            } else {
                break;
            }
        }

        terminal.draw(|frame| {
            frame.render_widget(
                LoadingLine::new(format!("loading / {}", lnum), LoadingLineState::Loading),
                frame.area(),
            );
        })?;
    }

    terminal.insert_before(1, |buf| {
        widgets::LoadingLine::new(
            Line::from(format!("finish fetching output / {}", lnum))
                .style(Style::default().white()),
            LoadingLineState::Success,
        )
        .render(buf.area, buf);
    })?;
    Ok(buffer)
}

async fn run(config: Config, read_source: ReadSource, show_lines: bool) -> Result<()> {
    let mut terminal = ratatui::init_with_options(TerminalOptions {
        viewport: Viewport::Inline(MAX_INLINE_VIEW as u16),
    });
    let output_data = get_source_data(&mut terminal, read_source, show_lines).await?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Args::parse();
    let config = match cli.config {
        Some(p) => Config::from_path(p)?,
        None => {
            let config_path = config::default_config_path()?;
            if config_path.exists() {
                Config::from_path(config_path)?
            } else {
                Config::default()
            }
        }
    };

    let res = run(
        config,
        cli.read_source.unwrap_or(ReadSource::Stdin),
        !cli.no_line_number,
    )
    .await;
    ratatui::restore();

    res
}
