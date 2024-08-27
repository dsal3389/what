use clap::Parser;
use config::Configuration;
use crossterm::{
    style::{self, Stylize},
    terminal::{disable_raw_mode, enable_raw_mode},
    QueueableCommand,
};
use std::{
    env, fmt,
    io::{stdout, Write},
    process::Stdio,
};

mod config;
mod terminal;

const SIGNATURE: &str = "---start---";

#[derive(Parser, Debug)]
struct Arguments {
    // how many lines to read for context
    #[arg(short, long, default_value_t = 500)]
    lines: u32,

    #[arg(short, long, default_value_t = false)]
    snippet: bool,

    #[arg(long, default_value_t = false)]
    last: bool,
}

struct TerminalCapture {
    lines: Vec<String>,
}

impl TerminalCapture {
    const MAX_SNIPPET_LENGTH: u16 = 16;

    async fn lines(n: u32) -> anyhow::Result<Self> {
        let lines = Self::tmux_capture(n).await?;
        Ok(Self { lines })
    }

    async fn last_command(n: u32) -> anyhow::Result<Self> {
        let lines = Self::tmux_capture(n).await?;

        for line in lines.iter().rev() {}
        Ok(Self { lines: vec![] })
    }

    fn print_snippet(&self) {
        let snippet = if self.lines.len() > Self::MAX_SNIPPET_LENGTH as usize {
            let end = self.lines.len() / 2 + (Self::MAX_SNIPPET_LENGTH as usize / 2);
            let start = end - (Self::MAX_SNIPPET_LENGTH as usize / 2);
            self.lines[start..end].join("\r\n")
        } else {
            self.lines.join("\r\n")
        };
        let mut stdout = stdout();

        let _ = stdout
            .queue(style::Print("\r\n...\r\n"))
            .and_then(|stdout| {
                stdout.queue(style::Print(format!("{}\r\n", snippet.italic().grey())))
            })
            .and_then(|stdout| stdout.queue(style::Print("\r\n...\r\n")));
        let _ = stdout.flush();
    }

    /// returns the captured terminal lines with tmux
    async fn tmux_capture(lines_count: u32) -> anyhow::Result<Vec<String>> {
        if env::var("TMUX").is_err() {
            anyhow::bail!("process can only run inside TMUX");
        }
        tokio::process::Command::new("tmux")
            .args(["capture-pane", "-T", "-pS", &format!("-{}", lines_count)])
            .stdout(Stdio::piped())
            .output()
            .await
            .map_err(anyhow::Error::new)
            .and_then(|result| {
                if !result.status.success() {
                    anyhow::bail!("tmux exited with error status");
                }
                String::from_utf8(result.stdout).map_err(anyhow::Error::new)
            })
            .and_then(|stdout| Ok(stdout.split_terminator('\n').map(String::from).collect()))
    }
}

impl fmt::Display for TerminalCapture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.lines.join("\n"))
    }
}

async fn gpt_diagnose(token: String, capture: String) -> anyhow::Result<String> {
    let client = reqwest::Client::new();
    let payload = serde_json::json!({
        "model": "gpt-4o",
        "messages": [
            {
                "role": "system",
                "content": "you are a smart assistent, you are inside a terminal don't response with markdown, give brief stringht to the point answer"
            },
            {
                "role": "user",
                "content": capture
            }
        ]
    });
    let response = client
        .post("https://api.openai.com/v1/chat/completions")
        .header("Content-Type", "application/json")
        .bearer_auth(token)
        .body(payload.to_string())
        .send()
        .await
        .map_err(anyhow::Error::new)?; // TODO better mapping

    response
        .json()
        .await
        .and_then(
            |data: serde_json::Value| Ok(data["choices"][0]["message"]["content"].to_string()),
        )
        .map_err(anyhow::Error::new)
}

async fn run() -> anyhow::Result<()> {
    let args = Arguments::parse();
    let config = Configuration::parse(None)
        .await
        .expect("couldn't parse config file");
    let _ = enable_raw_mode();

    let capture = if args.last {
        terminal::Loading::start(
            TerminalCapture::last_command(args.lines),
            "capturing last command output",
            "✨ last command output fetched",
            "couldn't fetch last command output",
        )
        .await
    } else {
        terminal::Loading::start(
            TerminalCapture::lines(args.lines),
            "capturing terminal output lines",
            "✨ output fetched",
            "couldn't fetch terminal output",
        )
        .await
    };

    if args.snippet {
        capture.print_snippet();
    }

    let diagnose = terminal::Loading::start(
        gpt_diagnose(config.token, capture.to_string()),
        "diagnosing with Chat-GPT",
        "done",
        "problem with GPT diagnostics",
    )
    .await;

    let _ = write!(stdout(), "{}", diagnose);
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    std::panic::set_hook(Box::new(|panic_info| {
        let _ = disable_raw_mode();
        eprintln!("🛑 unexpected error occured in program execution.");
        eprintln!("panic error: {panic_info}");
    }));
    run().await?;
    let _ = disable_raw_mode();
    Ok(())
}
