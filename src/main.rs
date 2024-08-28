use anyhow::Context;
use clap::{arg, Parser, ValueEnum};
use crossterm::{
    style::{PrintStyledContent, Stylize},
    terminal::{disable_raw_mode, enable_raw_mode},
    QueueableCommand,
};
use futures::StreamExt;
use reqwest::StatusCode;
use reqwest_eventsource::{Error::InvalidStatusCode, Event, EventSource, RequestBuilderExt};
use serde_json::{json, Value};
use std::{
    env, fmt,
    io::{stdout, Stdout, Write},
};

const PROMPT_REPLACE_STR: &'static str = ">>> ";

#[derive(Parser)]
struct Arguments {
    /// how many lines to capture for context
    #[arg(short, long, default_value_t = 500)]
    lines: u32,

    /// show the captured output
    #[arg(short, long, default_value_t = false)]
    show: bool,

    /// attach extra message to the sent data
    #[arg(short, long)]
    attach: Option<String>,

    /// how many command outputs to get, if non given then all
    /// captured lines will be sent
    #[arg(long)]
    last: Option<u32>,

    /// GPT model to use to get an answer
    #[arg(value_enum, default_value_t = DiagnosticModel::GPTTurbo)]
    model: DiagnosticModel,
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
        last_commands_lines.reverse();
        Ok(Self {
            lines: last_commands_lines,
        })
    }

    // capturing the terminal stdout with tmux
    async fn tmux_capture_lines(count: u32) -> anyhow::Result<Vec<String>> {
        if env::var("TMUX").is_err() {
            anyhow::bail!("process must run inside TMUX");
        }
        let result = tokio::process::Command::new("tmux")
            .args(&["capture-pane", "-T", "-pS", &format!("-{}", count)])
            .output()
            .await
            .context("couldn't spawn tmux process")?;

        if result.status.success() {
            String::from_utf8(result.stdout)
                .and_then(|output| Ok(output.split_terminator("\n").map(String::from).collect()))
                .context("couldn't read tmux capture pane")
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

        let result = tokio::process::Command::new(shell)
            .args(shell_arguments)
            .current_dir(env::current_dir().context("couldn't get the current working directory")?)
            .output()
            .await
            .context("couldn't fetch shell prompt")?;
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

    async fn write_to_screen(&self, stdout: &mut Stdout, extra_prompt: Option<String>) {
        let mut request_stream = self.openai_request_stream(extra_prompt);
        while let Some(event) = request_stream.next().await {
            match event {
                Ok(Event::Open) => {}
                Ok(Event::Message(message)) => {
                    let json_data: Value = serde_json::from_str(message.data.as_str()).unwrap();
                    let gpt_message = &json_data["choices"][0];

                    if gpt_message["finish_reason"] == "stop" {
                        let _ = write!(stdout, "\r\n");
                        break;
                    }

                    let _ = stdout.queue(PrintStyledContent(
                        gpt_message["delta"]["content"]
                            .to_string()
                            .trim_matches('"')
                            .cyan(),
                    ));
                    let _ = stdout.flush();
                }
                Err(err) => match err {
                    InvalidStatusCode(code, _) => {
                        let error_message = match code {
                            StatusCode::UNAUTHORIZED => "Incorrect API key provided",
                            StatusCode::TOO_MANY_REQUESTS => "Exceeded you current quota or too many requests, please check your plan and billin details",
                            StatusCode::FORBIDDEN => "Country, region, or territory not supported",
                            StatusCode::INTERNAL_SERVER_ERROR => "Server had an error while processing your request",
                            StatusCode::SERVICE_UNAVAILABLE => "The engine is currently overloaded, please try again later",
                            _ => "unexpected openai error occured"
                        };
                        panic!("[{}] {}", code, error_message);
                    }
                    err => {
                        panic!("unexpected openai error response: {:?}", err);
                    }
                },
            }
        }
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
    let terminal_capture = TerminalCapture::from_last_commands(args.lines, 1).await?;

    if args.show {
        stdout.queue(PrintStyledContent(
            format!("```\r\n{}\r\n```\r\n", terminal_capture.lines.join("\r\n"))
                .italic()
                .dark_grey(),
        ))?;
        let _ = stdout.flush();
    }

    let diagnostics = Diagnostics::new(
        env::var("OPENAI_TOKEN").unwrap(),
        args.model,
        &terminal_capture,
    );
    diagnostics.write_to_screen(&mut stdout, args.attach).await;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();

        eprintln!("{}", "unexpected error occured".red());
        default_panic(panic_info);
    }));

    let arguments = Arguments::parse();
    enable_raw_mode().context("couldn't enable raw mode")?;
    run(arguments)
        .await
        .inspect_err(|_| {
            let _ = disable_raw_mode();
        })
        .and_then(|_| {
            let _ = disable_raw_mode();
            Ok(())
        })
}
