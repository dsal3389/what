use std::pin::Pin;
use std::{env, fmt::Display};

use anyhow::{Context, Result};
use futures::{Stream, TryStreamExt};
use reqwest_eventsource::{Event, RequestBuilderExt};
use serde_json::json;

use super::{AgentClient, AgentEvent};
use crate::Config;

enum MessageRole {
    System,
    User,
    Assistant,
}

struct Message {
    role: MessageRole,
    message: String,
}

#[derive(Debug)]
pub struct OpenAIProvider {
    model: String,
    token: String,
}

impl OpenAIProvider {
    fn parse_event(event: Event) -> AgentEvent {
        match event {
            Event::Open => AgentEvent::Open,
            Event::Message(m) => {
                let json: serde_json::Value = serde_json::from_str(&m.data)
                    .expect("returned data from openai is not a valid json");
                let gpt_message = &json["choices"][0];

                if gpt_message["finish_reason"] == "stop" {
                    AgentEvent::End
                } else if let serde_json::Value::String(s) = &gpt_message["delta"]["content"] {
                    AgentEvent::Text(s.clone())
                } else {
                    panic!("couldn't parse openai api response, content doesn't look like expected json schema")
                }
            }
        }
    }
}

impl TryFrom<&Config> for OpenAIProvider {
    type Error = anyhow::Error;
    fn try_from(value: &Config) -> std::result::Result<Self, Self::Error> {
        let token = env::var("OPENAI_TOKEN")
            .or_else(|_| {
                value
                    .openai_api_token
                    .clone()
                    .context("couldn't get openai token from configuration file")
            })
            .context("couldn't get openai token from env variable `OPENAI_TOKEN`")?;
        let model = value
            .model
            .clone()
            .context("no openai model was given, use --model or add to configuration")?;
        Ok(OpenAIProvider { token, model })
    }
}

impl AgentClient for OpenAIProvider {
    fn request(&self, message: &str) -> Result<Pin<Box<dyn Stream<Item = Result<AgentEvent>>>>> {
        let request = reqwest::Client::builder()
            .build()?
            .post("https://api.openai.com/v1/chat/completions")
            .bearer_auth(&self.token)
            .json(&json!({
                "model": "gpt-3.5-turbo",
                "messages": [{
                    "role": "user",
                    "content": message,
                }],
                "stream": true
            }))
            .eventsource()?
            .map_ok(Self::parse_event)
            .map_err(|e| e.into());
        Ok(Box::pin(request))
    }
}

impl Display for OpenAIProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "openai-{}", self.model)
    }
}
