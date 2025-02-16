use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

use anyhow::Result;
use clap::{Parser, Subcommand};
use futures::StreamExt;

use ratatui::prelude::*;
use ratatui::text::ToLine;
use ratatui::{TerminalOptions, Viewport};

use tokio::fs::File;
use tokio::io::{self, stdin, AsyncBufReadExt, AsyncRead, BufReader};
use tokio::time::timeout;

mod agents;
mod config;
mod widgets;

use crate::agents::{Agent, AgentClient, AgentEvent, AgentProvider};
use crate::config::Config;
use crate::widgets::LoadingLine;

#[derive(Parser, Debug)]
struct Args {
    #[arg(long = "no-line-number", default_value_t = false)]
    no_show_lines: bool,

    #[arg(short, long)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    action: Option<CliAction>,
}

#[derive(Subcommand, Debug)]
enum CliAction {
    Config {
        #[command(subcommand)]
        action: Option<CliConfigAction>,
    },
    Read {
        #[command(subcommand)]
        source: Option<ReadSource>,
    },
}

impl Default for CliAction {
    fn default() -> Self {
        Self::Read {
            source: Some(ReadSource::default()),
        }
    }
}

#[derive(Subcommand, Debug, Default)]
enum CliConfigAction {
    #[default]
    View,
    Set,
}

#[derive(Subcommand, Debug, Default)]
enum ReadSource {
    #[default]
    Stdin,
    File {
        path: PathBuf,
    },
}

impl ReadSource {
    /// returns a BufReader on the source reader
    async fn reader(&self) -> io::Result<BufReader<Pin<Box<dyn AsyncRead>>>> {
        let reader: Pin<Box<dyn AsyncRead>> = match self {
            Self::Stdin => Box::pin(stdin()),
            Self::File { path } => Box::pin(File::open(path).await?),
        };
        Ok(BufReader::new(reader))
    }
}

/// setup the terminal with predefined arguments
fn setup_terminal() -> Terminal<impl Backend> {
    ratatui::init_with_options(TerminalOptions {
        viewport: Viewport::Inline(1),
    })
}

/// get the the output from the given read source and print the
/// relavent information to the screen
async fn get_source_data(
    terminal: &mut Terminal<impl Backend>,
    read_source: ReadSource,
    show_lines: bool,
) -> Result<Vec<String>> {
    let reader = read_source.reader().await?;
    let mut lines = reader.lines();
    let mut buffer = Vec::new();
    let mut lnum = 0_usize;

    loop {
        // prevent the `next_line` from blocking the print to screen
        if let Ok(line_res) = timeout(Duration::from_millis(300), lines.next_line()).await {
            if let Some(line) = line_res? {
                lnum += 1;

                if lnum == 200 {
                    // prevent printing to stdout slow performance
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

        // print the loading bar every frame if we have
        // or don't have data read
        terminal.draw(|frame| {
            frame.render_widget(
                LoadingLine::new(format!("loading / {}", lnum)),
                frame.area(),
            );
        })?;
    }

    terminal.insert_before(1, |buf| {
        widgets::LoadingLine::new(
            Line::from(format!("finish fetching output / {}", lnum))
                .style(Style::default().white()),
        )
        .sucess()
        .render(buf.area, buf);
    })?;
    Ok(buffer)
}

async fn agent_response(
    terminal: &mut Terminal<impl Backend>,
    agent: Agent,
    message: &str,
) -> Result<()> {
    let mut stream = agent.request(message)?;
    loop {
        let event = timeout(Duration::from_millis(300), stream.next()).await;
        if let Ok(event) = event {
            if let Some(event) = event {
                match event? {
                    AgentEvent::Text(text) => {
                        terminal.insert_before(1, |buf| {
                            Line::from(text.as_str()).render(buf.area, buf);
                        })?;
                    }
                    AgentEvent::Open => {
                        terminal.insert_before(1, |buf| {
                            Line::from(agent.to_string())
                                .style(Style::default().green())
                                .render(buf.area, buf)
                        })?;
                    }
                    AgentEvent::End => break,
                }
            } else {
                break;
            }
        }
        terminal.draw(|frame| {
            frame.render_widget(widgets::LoadingLine::new("loading..."), frame.area());
        })?;
    }
    Ok(())
}

async fn suggest(config: Config, read_source: ReadSource, show_lines: bool) -> Result<()> {
    let mut terminal = setup_terminal();
    let agent = Agent::try_from(&config)?;
    let lines = get_source_data(&mut terminal, read_source, show_lines).await?;
    let content = String::from_iter(lines);
    agent_response(&mut terminal, agent, &content).await
}

async fn cfg_view(
    terminal: &mut Terminal<impl Backend>,
    cfg: Config,
    show_lines: bool,
) -> Result<()> {
    for line in serde_json::to_string_pretty(&cfg)?.lines() {
        terminal.insert_before(1, |buf| {
            Text::from(line).render(buf.area, buf);
        })?;
    }
    Ok(())
}

async fn cfg_set(terminal: &mut Terminal<impl Backend>, cfg: Config) -> Result<()> {
    Ok(())
}

async fn run(terminal: &mut Terminal<impl Backend>, args: Args) -> Result<()> {
    let cfg_path = args.config.unwrap_or_else(config::default_config_path);

    if !cfg_path.exists() {
        terminal.insert_before(1, |buf| {
            Line::from(format!(
                "no config file found, creating default config file at {}",
                cfg_path.display()
            ))
            .style(Style::default().light_yellow())
            .render(buf.area, buf)
        })?;
        Config::default().save(cfg_path)?;
        return Ok(());
    }

    let cfg = Config::from_path(&cfg_path)?;

    match args.action.unwrap_or_default() {
        CliAction::Config { action } => match action.unwrap_or_default() {
            CliConfigAction::View => cfg_view(terminal, cfg, !args.no_show_lines).await,
            CliConfigAction::Set => cfg_set(terminal, cfg).await,
        },
        CliAction::Read { source } => {
            suggest(cfg, source.unwrap_or_default(), !args.no_show_lines).await
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Args::parse();
    let mut terminal = setup_terminal();

    let res = run(&mut terminal, cli).await.inspect_err(|err| {
        terminal
            .insert_before(1, |buf| {
                err.to_line()
                    .style(Style::default().red())
                    .render(buf.area, buf);
            })
            .unwrap();
    });
    ratatui::restore();
    res
}
