use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Duration;

use anyhow::Result;
use clap::{Parser, Subcommand};
use futures::StreamExt;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::tty::IsTty;

use ratatui::prelude::*;
use ratatui::text::{ToLine, ToSpan};
use ratatui::widgets::{Block, LineGauge, Paragraph};
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
    /// don't print line numbers when fetching content (relevant for specific cases)
    #[arg(long = "no-line-number", default_value_t = false)]
    no_show_lines: bool,

    /// define path to configuration file instead of using default
    #[arg(short, long)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    action: Option<CliAction>,
}

#[derive(Subcommand, Debug)]
enum CliAction {
    /// configuration actions
    Config {
        #[command(subcommand)]
        action: Option<CliConfigAction>,
    },

    /// read content from a given file
    File {
        /// path to the file
        path: PathBuf,
    },

    /// read content from stdin (default)
    Stdin {
        /// read only stderr output (works only in pipe mode)
        #[arg(long = "stderr", default_value_t = false)]
        only_stderr: bool,
    },
}

impl Default for CliAction {
    fn default() -> Self {
        CliAction::Stdin { only_stderr: false }
    }
}

#[derive(Subcommand, Debug, Default)]
enum CliConfigAction {
    /// view the configuration content and path
    #[default]
    View,

    /// set attributes in the configuration file
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
            CliAction::Stdin { only_stderr } => Self::Stdin,
            CliAction::File { path } => Self::File { path },
            _ => panic!("couldn't convert {:?} to ReadSource", value), // should panic because this is likey a bug that we got here
        }
    }
}

#[allow(dead_code)]
fn fixed_bottom(height: u16, area: Rect) -> Rect {
    Rect {
        x: area.x,
        y: area.bottom() - height,
        width: area.width,
        height,
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

    terminal.insert_before(1, |buf| {
        widgets::LoadingLine::new(Line::from(format!("finish fetching output / {}", lnum)))
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
    // TODO: find a better way to manipulate existing terminal
    // instead of initializing a new one
    // let mut terminal = ratatui::init_with_options(TerminalOptions {
    //    viewport: Viewport::Inline(2),
    // });

    let mut stream = agent.request(message)?;
    let mut buffer = String::new();

    loop {
        // NOTE: although this timeout doesn't required because
        // there is nothing that required to run concurrently, this is for the future
        // to be able to print loading line with waiting for events
        let event = timeout(Duration::from_millis(300), stream.next()).await;
        if let Ok(event) = event {
            match event {
                Some(event) => match event? {
                    AgentEvent::Text(text) => {
                        let width = terminal.get_frame().area().width;

                        // if the line is too long to be printed in the current terminal width, then
                        // we break the line and print `\` to indicate line break
                        if (buffer.len() + text.len()) as u16 >= width {
                            terminal.insert_before(1, |buf| {
                                Line::from(vec![
                                    buffer.to_span(),
                                    Span::from(" \\").style(Style::default().yellow()),
                                ])
                                .render(buf.area, buf);
                            })?;

                            buffer.clear();
                        }

                        // if the agent sent a text that should indicate end of line, we should
                        // flush whatever we have in current buffer, and clear it for the next lin
                        match text.strip_suffix('\n') {
                            Some(text) => {
                                buffer.push_str(text);
                                terminal.insert_before(1, |buf| {
                                    buffer.to_line().render(buf.area, buf);
                                })?;
                                buffer.clear();
                            }
                            None => buffer.push_str(&text),
                        }
                    }
                    AgentEvent::Open => {
                        terminal.insert_before(1, |buf| {
                            LineGauge::default()
                                .label(format!(" {}", agent.to_string()))
                                .style(Style::default().yellow())
                                .render(buf.area, buf);
                        })?;
                    }
                    AgentEvent::End => {
                        // before we close the chat we flush what ever we
                        // have in the buffer to the terminal
                        if !buffer.is_empty() {
                            terminal.insert_before(1, |buf| {
                                buffer.to_line().render(buf.area, buf);
                            })?;
                        }
                        break;
                    }
                },
                None => break,
            }
        }

        // prints whatever we have currently in our buffer and also
        // draw the loading line, indicating agent is still responding
        terminal.draw(|frame| {
            frame.render_widget(buffer.to_line(), frame.area());
            // frame.render_widget(
            //     widgets::LoadingLine::new("loading..."),
            //     fixed_bottom(1, frame.area()),
            // );
        })?;
    }
    Ok(())
}

async fn prompt_user(terminal: &mut Terminal<impl Backend>) -> Result<Option<String>> {
    terminal.insert_before(1, |buf| {
        LineGauge::default()
            .label(" YOU")
            .style(Style::default().cyan())
            .render(buf.area, buf);
    })?;

    let frame_area = terminal.get_frame().area();
    let mut buffer = String::with_capacity(frame_area.width as usize);
    let mut cursor_pos = 0_u16;

    terminal.draw(|frame| {
        frame.render_widget(
            widgets::InputLine::new("write message...", &buffer),
            frame.area(),
        );
        frame.set_cursor_position((0, frame.area().y));
    })?;

    loop {
        if let Event::Key(key) = event::read()? {
            match key.kind {
                KeyEventKind::Release => match key.code {
                    _ => {}
                },
                KeyEventKind::Press => match key.code {
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        terminal.insert_before(1, |buf| {
                            " // CTRL + C break"
                                .to_line()
                                .style(Style::default().dark_gray().italic())
                                .render(buf.area, buf);
                        })?;
                        break Ok(None);
                    }
                    KeyCode::Char(c) => {
                        buffer.insert(cursor_pos as usize, c);
                        cursor_pos += 1;
                    }
                    KeyCode::Right if cursor_pos < buffer.len() as u16 => {
                        cursor_pos += 1;
                    }
                    KeyCode::Left => {
                        cursor_pos = cursor_pos.saturating_sub(1);
                    }
                    KeyCode::Backspace if !buffer.is_empty() => {
                        cursor_pos = cursor_pos.saturating_sub(1);
                        buffer.remove(cursor_pos as usize);
                    }
                    KeyCode::Enter if !buffer.is_empty() => {
                        terminal.insert_before(1, |buf| {
                            buffer.to_line().render(buf.area, buf);
                        })?;
                        break Ok(Some(buffer));
                    }
                    _ => continue,
                },

                _ => continue,
            };

            terminal.draw(|frame| {
                frame.render_widget(
                    widgets::InputLine::new("write message...", &buffer),
                    frame.area(),
                );
                frame.set_cursor_position((cursor_pos, frame.area().y));
            })?;
        }
    }
}

async fn interactive_chat(terminal: &mut Terminal<impl Backend>, agent: Agent) -> Result<()> {
    while let Some(message) = prompt_user(terminal).await? {
        agent_response(terminal, &agent, &message).await?;
    }
    Ok(())
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
            let source = ReadSource::from(read_source);
            let agent = Agent::try_from(&cfg)?;

            match source {
                ReadSource::Stdin if !stdin().is_tty() => {
                    let data = get_source_data(terminal, source, !args.no_show_lines).await?;
                    agent_response(terminal, &agent, &data).await
                }
                ReadSource::File { .. } => {
                    let data = get_source_data(terminal, source, !args.no_show_lines).await?;
                    agent_response(terminal, &agent, &data).await
                }
                ReadSource::Stdin => interactive_chat(terminal, agent).await,
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
