use anyhow::Context;
use clap::{arg, Parser, Subcommand, ValueEnum};
use crossterm::{
    style::{PrintStyledContent, Stylize},
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
    QueueableCommand,
};
use futures::StreamExt;
use reqwest::StatusCode;
use reqwest_eventsource::{Error::InvalidStatusCode, Event, EventSource, RequestBuilderExt};
use serde_json::{json, Value};
use std::{
    env, fmt,
    io::{stdin, stdout, BufRead, Read, Stdout, Write},
};

const PROMPT_REPLACE_STR: &'static str = ">>> ";

#[derive(Parser)]
struct Arguments {
    /// don't display captured output (won't ask for output confirmation)
    #[arg(short, long, default_value_t = false)]
    quite: bool,

    /// don't ask for confirmation
    #[arg(short, long, default_value_t = false)]
    confirm: bool,

    /// attach extra message to the sent data
    #[arg(short, long)]
    attach: Option<String>,

    /// GPT model to use for the diagnoses
    #[arg(value_enum, default_value_t = DiagnosticModel::GPTTurbo)]
    model: DiagnosticModel,

    #[command(subcommand)]
    commands: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// spawn a subprocess and capture its output
    /// if exit status code is fail
    Execute {
        #[arg(action = clap::ArgAction::Append)]
        /// the command to execute
        command: String,

        /// continue even if processes didn't exist with
        /// error code
        #[arg(short, long, default_value_t = false)]
        force: bool,
    },
    /// capture number of lines from the terminal,
    /// supported only inside TMXU pane
    Lines {
        /// how many lines to capture from the terminal
        count: u32,
    },
    /// capture only N commands
    Last {
        /// how many last commands to capture
        count: u8,

        /// how many lines to read that include
        /// all those captured commands
        #[arg(short, long, default_value_t = 500)]
        lines: u32,
    },
}

struct TerminalCapture {
    lines: Vec<String>,
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum DiagnosticModel {
    GPTOmni,
    GPTOmniMini,
    GPTTurbo,
}

struct Diagnostics<'a> {
    token: String,
    capture: &'a TerminalCapture,
    model: DiagnosticModel,
}

impl TerminalCapture {
    async fn from_lines(count: u32) -> anyhow::Result<Self> {
        let lines = Self::tmux_capture_lines(count).await?;
        Ok(Self { lines })
    }

    /// return `TerminalCapture` struct containing lines from the
    /// last N commands, this is done by taking the terminal prompt, going from bottom to top
    /// and everytime we hit the prompt in the terminal output, it is a command
    /// for example, read in reverse
    /// ```console
    /// >>> ls -al  # hit, a line that start with prompt, all we read below its a output
    /// .       --- user:user bla bla  # """"
    /// ..      --- user:user bla bla  # """"
    /// foo.txt --- user:user bla bal  # outputline becuase doesn't start with prompt
    /// >>>  # line that starts with prompt
    /// ```
    ///
    async fn from_last_commands(lines_count: u32, commands_count: u8) -> anyhow::Result<Self> {
        let prompt = Self::prompt().await?;
        let mut last_commands_lines = Vec::new();
        let mut prompt_hit = 0u8;

        // going in reverse order on the lines to get the last commands output
        for line in Self::tmux_capture_lines(lines_count).await?.iter().rev() {
            if line.starts_with(&prompt) {
                prompt_hit += 1;

                // first prompt hit will always be the prompt that executed
                // the current process (the `what` tool)
                if prompt_hit > 1 {
                    let line = line.replace(&prompt, PROMPT_REPLACE_STR);
                    last_commands_lines.push(line.clone());
                }

                if prompt_hit == commands_count + 1 {
                    break;
                }
            } else if prompt_hit > 0 {
                last_commands_lines.push(line.clone());
            }
        }

        // reorder the lines back to the original
        // order before returning
        last_commands_lines.reverse();
        Ok(Self {
            lines: last_commands_lines,
        })
    }

    async fn from_command(command: String, force: bool) -> anyhow::Result<TerminalCapture> {
        let parts = command.split_whitespace().collect::<Vec<&str>>();
        let (command, arguments) = parts.split_at(1);

        let result = tokio::process::Command::new(command[0])
            .args(arguments)
            .output()
            .await
            .with_context(|| format!("couldn't spawn process `{}`", command[0]))?;

        if !force && result.status.success() {
            anyhow::bail!(
                "process `{}` didn't exit with error status code, aborting",
                command[0]
            );
        }

        // capture both child process stdout and stderr
        let lines = result
            .stdout
            .lines()
            .chain(result.stderr.lines())
            .map(|l| l.expect("subprocess output contains invalid utf-8 chars"))
            .collect::<Vec<String>>();
        Ok(Self { lines })
    }

    // capturing the terminal stdout with tmux
    async fn tmux_capture_lines(count: u32) -> anyhow::Result<Vec<String>> {
        if env::var("TMUX").is_err() {
            anyhow::bail!("process must run inside TMUX");
        }

        // take the current terminal output using TMUX
        let result = tokio::process::Command::new("tmux")
            .args(&["capture-pane", "-T", "-pS", &format!("-{}", count)])
            .output()
            .await
            .context("couldn't spawn tmux process")?;

        if result.status.success() {
            Ok(result
                .stdout
                .lines()
                .map(|l| l.expect("invalid output in tmux"))
                .collect())
        } else {
            anyhow::bail!("tmux process existed with error status code");
        }
    }

    // returns the terminal processed terminal prompt, this helps
    // clean the terminal output and serialize it properly, the function uses
    // standard env variables that are expected to be set, $SHELL and $PS1
    async fn prompt() -> anyhow::Result<String> {
        let shell = env::var("SHELL").map_err(|_| {
            anyhow::anyhow!("couldn't get the current shell from environment variable `SHELL`")
        })?;
        let shell_arguments = match shell.split_terminator('/').last().unwrap_or("") {
            "zsh" => &["-i", "-c", "print -P $PS1"],
            "bash" | "sh" => &["-i", "-c", "echo -e \"${PS1@P}\""],
            _ => anyhow::bail!(format!("unsupported shell to get prompt from `{}`", shell)),
        };

        // execute the shell fetch in the current working directory because
        // some users may have something like `git` plugins that shows
        // their current branch in the prompt, so where we are does matter
        let result = tokio::process::Command::new(shell)
            .args(shell_arguments)
            .current_dir(env::current_dir().context("couldn't get the current working directory")?)
            .output()
            .await
            .context("couldn't fetch shell prompt")?;

        // get the output from the process output, we always take the last
        // line because we spawned interactive shell for zsh and bash, and users
        // might have banners at the top when starting a shell
        let raw_prompt = String::from_utf8(result.stdout)
            .map_err(anyhow::Error::new)
            .and_then(|s| {
                s.split_terminator('\n')
                    .last()
                    .ok_or(anyhow::anyhow!(
                        "couldn't get last line in terminal prompt output"
                    ))
                    .and_then(|s| Ok(s.to_string()))
            })?;
        Ok(strip_ansi_escapes::strip_str(raw_prompt))
    }
}

impl<'a> Diagnostics<'a> {
    fn new(token: String, model: DiagnosticModel, capture: &'a TerminalCapture) -> Self {
        Self {
            token,
            model,
            capture,
        }
    }

    async fn write_to_screen(
        &self,
        stdout: &mut Stdout,
        extra_prompt: Option<String>,
    ) -> anyhow::Result<()> {
        let mut request_stream = self.openai_request_stream(extra_prompt);
        while let Some(event) = request_stream.next().await {
            match event {
                Ok(Event::Open) => {}
                Ok(Event::Message(message)) => {
                    let json_data: Value = serde_json::from_str(message.data.as_str()).unwrap();
                    let gpt_message = &json_data["choices"][0];

                    if gpt_message["finish_reason"] == "stop" {
                        write!(stdout, "\r\n")?;
                        break;
                    }

                    stdout
                        .queue(PrintStyledContent(
                            gpt_message["delta"]["content"]
                                .to_string()
                                .trim_matches('"')
                                .cyan(),
                        ))?
                        .flush()?;
                }
                Err(err) => match err {
                    InvalidStatusCode(code, _) => {
                        // messages are from the openai documentation
                        let error_message = match code {
                            StatusCode::UNAUTHORIZED => "Incorrect API key provided",
                            StatusCode::TOO_MANY_REQUESTS => "Exceeded you current quota or too many requests, please check your plan and billin details",
                            StatusCode::FORBIDDEN => "Country, region, or territory not supported",
                            StatusCode::INTERNAL_SERVER_ERROR => "Server had an error while processing your request",
                            StatusCode::SERVICE_UNAVAILABLE => "The engine is currently overloaded, please try again later",
                            _ => "unexpected openai error occured"
                        };
                        anyhow::bail!("[{}] {}", code.as_u16(), error_message)
                    }
                    err => {
                        anyhow::bail!("unexpected openai error response: {:?}", err)
                    }
                },
            }
        }
        Ok(())
    }

    fn openai_request_stream(&self, extra_prompt: Option<String>) -> EventSource {
        let payload = json!({
            "model": self.model.to_string(),
            "messages": [
                {
                    "role": "system",
                    "content": "you are a helpful assistant, you get commands outputs and you diagnose what was the issue and given a solution, do not send markdown text"
                },
                {"role": "user", "content": format!("{}\n{}", self.capture.to_string(), extra_prompt.unwrap_or("".to_string()))}
            ],
            "stream": true
        });
        reqwest::Client::new()
            .post("https://api.openai.com/v1/chat/completions")
            .header("Content-Type", "application/json")
            .bearer_auth(&self.token)
            .body(payload.to_string())
            .eventsource()
            .unwrap()
    }
}

impl fmt::Display for TerminalCapture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.lines.join("\n"))
    }
}

impl fmt::Display for DiagnosticModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GPTOmni => write!(f, "gpt-4o"),
            Self::GPTOmniMini => write!(f, "gpt-4o-mini"),
            Self::GPTTurbo => write!(f, "gpt-3.5-turbo"),
        }
    }
}

async fn run(args: Arguments) -> anyhow::Result<()> {
    let mut stdout = stdout();
    let terminal_capture = match args.commands {
        Commands::Execute { command, force } => {
            TerminalCapture::from_command(command, force).await?
        }
        Commands::Lines { count } => TerminalCapture::from_lines(count).await?,
        Commands::Last { count, lines } => {
            TerminalCapture::from_last_commands(lines, count).await?
        }
    };

    if terminal_capture.lines.len() == 0 {
        anyhow::bail!(
            "couldn't capture anything from the terminal, is SHELL env variable set correctly?"
        );
    }

    if !args.quite {
        stdout
            .queue(PrintStyledContent(
                format!("```\r\n{}\r\n```\r\n", terminal_capture.lines.join("\r\n"))
                    .italic()
                    .dark_grey(),
            ))?
            .queue(PrintStyledContent(
                format!("captured {} lines\r\n", terminal_capture.lines.len())
                    .italic()
                    .dark_grey(),
            ))?
            .flush()?;

        if !args.confirm {
            let mut buf = [0u8; 1];
            stdout
                .queue(PrintStyledContent("confirm output [Y/n]".bold().white()))?
                .flush()?;

            stdin()
                .read_exact(&mut buf)
                .context("couldn't read from stdin")?;

            if buf[0].to_ascii_lowercase() != 'y' as u8 {
                stdout
                    .queue(Clear(ClearType::CurrentLine))?
                    .queue(PrintStyledContent("\raborting...\r\n".red().bold()))?
                    .flush()?;
                return Ok(());
            }

            stdout
                .queue(Clear(ClearType::CurrentLine))?
                .queue(PrintStyledContent("\routput confirmed\r\n".green().bold()))?
                .flush()?;
        }
    }

    let diagnostics = Diagnostics::new(
        env::var("OPENAI_TOKEN").unwrap(),
        args.model,
        &terminal_capture,
    );
    diagnostics
        .write_to_screen(&mut stdout, args.attach)
        .await?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();

        eprintln!("{}", "panic error occured".red().bold().underlined());
        default_panic(panic_info);
    }));

    let arguments = Arguments::parse();
    enable_raw_mode().context("couldn't enable raw mode")?;
    run(arguments)
        .await
        .inspect_err(|_| {
            let _ = disable_raw_mode();
        })
        .map_err(|err| {
            anyhow::anyhow!(PrintStyledContent(format!("{}", err).red().bold()).to_string())
        })
        .and_then(|_| {
            let _ = disable_raw_mode();
            Ok(())
        })
}
