use std::panic;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Duration;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use futures::StreamExt;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::tty::IsTty;

use ratatui::prelude::*;
use ratatui::text::{ToLine, ToSpan};
use ratatui::widgets::LineGauge;
use ratatui::{TerminalOptions, Viewport};

use tokio::fs::File;
use tokio::io::{self, stdin, AsyncBufReadExt, AsyncRead, BufReader};
use tokio::time::timeout;
use widgets::{InputLine, InputType};

mod agents;
mod config;
mod widgets;

use crate::agents::{Agent, AgentEvent};
use crate::config::Config;
use crate::widgets::LoadingLine;

#[derive(Parser, Debug)]
#[command(version)]
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

#[derive(Subcommand, Debug, Default)]
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
    #[default]
    Stdin,
}

#[derive(Subcommand, Debug, Default)]
enum CliConfigAction {
    /// view the configuration content and path
    #[default]
    View,

    /// set attributes in the configuration file
    Set {
        #[arg(value_enum)]
        option: CliConfigSetOptions,
    },
}

#[derive(Clone, Debug, ValueEnum)]
enum CliConfigSetOptions {
    /// set openai token instead of setting it as environment variable
    OpenaiToken,

    /// set the default provider and model
    DefaultProvider,
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

/// convert CliAction into a ReadSource, some CliActions cannot be converted into a ReadSource
/// so in that case the trait will panic, it is expected to pass only convertable CliActions
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

    terminal.insert_before(1, |buf| {
        widgets::LoadingLine::new(Line::from(format!("finish fetching output / {}", lnum)))
            .sucess()
            .render(buf.area, buf);
    })?;
    Ok(buffer)
}

async fn agent_response(
    terminal: &mut Terminal<impl Backend>,
    agent: &mut Agent,
    message: &str,
) -> Result<()> {
    let provider = agent.get_provider();
    let provider_name = provider.display_name();

    let mut stream = provider.request(message)?;
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
                                .label(format!(" {}", provider_name))
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
    InputLine::readline(terminal, " write message...", InputType::Clear)
}

async fn interactive_chat(terminal: &mut Terminal<impl Backend>, mut agent: Agent) -> Result<()> {
    while let Some(message) = prompt_user(terminal).await? {
        agent_response(terminal, &mut agent, &message).await?;
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

async fn cfg_set(
    terminal: &mut Terminal<impl Backend>,
    option: CliConfigSetOptions,
    cfg_path: &Path,
    mut cfg: Config,
) -> Result<()> {
    match option {
        CliConfigSetOptions::OpenaiToken => {
            terminal.insert_before(1, |buf| {
                LineGauge::default()
                    .label("openai-token")
                    .render(buf.area, buf);
            })?;

            if let Some(token) =
                InputLine::readline(terminal, "openai-api-token...", InputType::Secure)?
            {
                cfg.openai_api_token = Some(token);
            }
        }
        CliConfigSetOptions::DefaultProvider => {
            terminal.insert_before(3, |buf| {
                Text::from(vec![
                    "possible options are:".to_line(),
                    "\topenai".to_line(),
                ])
                .render(buf.area, buf);
            })?;
            if let Some(provider) = InputLine::readline(
                terminal,
                "provider name...",
                InputType::WithPrefix(" \u{2713}"),
            )? {
                cfg.provider = Some(provider.as_str().try_into()?)
            }

            terminal.insert_before(1, |buf| {
                LineGauge::default()
                    .label("model name")
                    .render(buf.area, buf);
            })?;
            if let Some(model) = InputLine::readline(
                terminal,
                "model name...",
                InputType::WithPrefix(" \u{2713}"),
            )? {
                cfg.model = Some(model);
            }
        }
    };
    cfg.save(cfg_path)
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

    // call the correct function based on the given cli arguments
    match args.action.unwrap_or_default() {
        CliAction::Config { action } => match action.unwrap_or_default() {
            CliConfigAction::View => cfg_view(terminal, &cfg_path, cfg, !args.no_show_lines).await,
            CliConfigAction::Set { option } => cfg_set(terminal, option, &cfg_path, cfg).await,
        },
        read_source => {
            // `ReadSource::from` is not expected to fail here, it if fails and
            // panics it is a bug
            let source = ReadSource::from(read_source);
            let mut agent = Agent::try_from(&cfg)?;

            match source {
                // if stdin is not tty, it means the data is piped to the program
                // and we need to read all the streamed data and not open interactive mode
                ReadSource::Stdin if !stdin().is_tty() => {
                    let data = get_source_data(terminal, source, !args.no_show_lines).await?;
                    agent_response(terminal, &mut agent, &data).await
                }
                ReadSource::File { .. } => {
                    let data = get_source_data(terminal, source, !args.no_show_lines).await?;
                    agent_response(terminal, &mut agent, &data).await
                }
                // if stdin is tty, it means no data is piped to the program so
                // it is expected to open interactive chat
                ReadSource::Stdin => interactive_chat(terminal, agent).await,
            }
        }
    }
}

fn install_panic_hook() {
    let original_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        ratatui::restore();
        println!("!!!");
        println!("unexpected error occured, this might be a bug\n");

        original_hook.as_ref()(panic_info);
    }));
}

#[tokio::main]
async fn main() -> Result<()> {
    install_panic_hook();
    let cli = Args::parse();
    let mut terminal = setup_terminal();

    if let Err(err) = run(&mut terminal, cli).await {
        terminal
            .insert_before(1, |buf| {
                err.to_line()
                    .style(Style::default().red().bold())
                    .render(buf.area, buf);
            })
            .unwrap();
    }

    ratatui::restore();
    Ok(())
}
