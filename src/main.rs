use std::io::{stdout, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Duration;

use anyhow::Result;
use clap::{Parser, Subcommand};
use crossterm::tty::IsTty;
use futures::StreamExt;

use ratatui::prelude::*;
use ratatui::text::ToLine;
use ratatui::widgets::LineGauge;
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

#[derive(Subcommand, Debug, Default)]
enum CliAction {
    Config {
        #[command(subcommand)]
        action: Option<CliConfigAction>,
    },
    File {
        path: PathBuf,
    },
    #[default]
    Stdin,
}

#[derive(Subcommand, Debug, Default)]
enum CliConfigAction {
    #[default]
    View,
    Set,
}

#[derive(Debug)]
enum ReadSource {
    Stdin,
    File { path: PathBuf },
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

impl From<CliAction> for ReadSource {
    fn from(value: CliAction) -> Self {
        match value {
            CliAction::Stdin => Self::Stdin,
            CliAction::File { path } => Self::File { path },
            _ => panic!("couldn't convert {:?} to ReadSource", value), // should panic because this is likey a bug that we got here
        }
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
    source: ReadSource,
    show_lines: bool,
) -> Result<String> {
    let reader = source.reader().await?;
    let mut lines = reader.lines();
    let mut buffer = String::new();
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
                                Span::from(format!(" {:4} \u{2502} ", lnum)),
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
                buffer.push_str(&line);
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

    dbg!(lnum);
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
    agent: &Agent,
    message: &str,
) -> Result<()> {
    let mut stream = agent.request(message)?;
    let mut buffer = String::new();
    let mut stdout = stdout();

    loop {
        let event = timeout(Duration::from_millis(300), stream.next()).await;
        if let Ok(event) = event {
            match event {
                Some(event) => match event? {
                    AgentEvent::Text(text) => {
                        stdout.write(text.as_bytes()).unwrap();
                        stdout.flush().unwrap();
                    }
                    AgentEvent::Open => {
                        terminal.insert_before(1, |buf| {
                            Line::from(agent.to_string())
                                .style(Style::default().green())
                                .render(buf.area, buf)
                        })?;
                    }
                    AgentEvent::End => break,
                },
                None => break,
            }
        }
        // terminal.draw(|frame| {
        //     frame.render_widget(widgets::LoadingLine::new("loading..."), frame.area());
        // })?;
    }
    Ok(())
}

async fn interactive_chat(terminal: &mut Terminal<impl Backend>, agent: Agent) -> Result<()> {
    todo!()
}

/// prints the loaded configuration to screen
/// in json format
async fn cfg_view(
    terminal: &mut Terminal<impl Backend>,
    cfg_path: &Path,
    cfg: Config,
    show_lines: bool,
) -> Result<()> {
    terminal.insert_before(1, |buf| {
        LineGauge::default()
            .label(cfg_path.display().to_line())
            .green()
            .render(buf.area, buf);
    })?;

    let json_content = serde_json::to_string_pretty(&cfg)?;
    let json_lines: Vec<String> = json_content.lines().map(|s| s.to_string()).collect();

    terminal.insert_before(json_lines.len() as u16, |buf| {
        json_lines
            .iter()
            .enumerate()
            .map(|(i, line)| {
                let spans: &[Span] = if show_lines {
                    &[Span::from(format!(" {:4} \u{2502} ", i)), Span::from(line)]
                } else {
                    &[Span::from(line)]
                };
                Line::from(spans.to_vec())
            })
            .collect::<Text>()
            .render(buf.area, buf);
    })?;
    Ok(())
}

async fn cfg_set(terminal: &mut Terminal<impl Backend>, cfg: Config) -> Result<()> {
    Ok(())
}

async fn cfg_create_default(
    terminal: &mut Terminal<impl Backend>,
    cfg_path: &Path,
) -> Result<Config> {
    terminal.insert_before(1, |buf| {
        Line::from(format!(
            "no config file found, creating default config file at {}",
            cfg_path.display()
        ))
        .style(Style::default().light_yellow())
        .render(buf.area, buf)
    })?;

    let cfg = Config::default();
    cfg.save(cfg_path)?;
    Ok(cfg)
}

async fn run(terminal: &mut Terminal<impl Backend>, args: Args) -> Result<()> {
    let cfg_path = args.config.unwrap_or_else(config::default_config_path);
    let cfg = if cfg_path.exists() {
        Config::from_path(&cfg_path)?
    } else {
        cfg_create_default(terminal, &cfg_path).await?
    };

    match args.action.unwrap_or_default() {
        CliAction::Config { action } => match action.unwrap_or_default() {
            CliConfigAction::View => cfg_view(terminal, &cfg_path, cfg, !args.no_show_lines).await,
            CliConfigAction::Set => cfg_set(terminal, cfg).await,
        },
        read_source => {
            let agent = Agent::try_from(&cfg)?;

            if !stdin().is_tty() {
                let data =
                    get_source_data(terminal, read_source.into(), !args.no_show_lines).await?;
                agent_response(terminal, &agent, &data).await
            } else {
                interactive_chat(terminal, agent).await
            }
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
