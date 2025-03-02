use std::borrow::Cow;
use std::collections::VecDeque;
use std::env;
use std::pin::Pin;

use anyhow::{Context, Result};
use futures::{Stream, TryStreamExt};
use reqwest_eventsource::{Event, RequestBuilderExt};

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{AgentEvent, AgentProvider};
use crate::Config;

#[derive(Debug, Serialize)]
enum MessageRole {
    System,
    User,
    Assistant,
}

#[derive(Debug, Serialize)]
struct Message {
    role: MessageRole,
    message: String,
}

impl Message {
    fn new(role: MessageRole, message: Cow<str>) -> Message {
        Message {
            role,
            message: message.into_owned(),
        }
    }
}

#[derive(Debug)]
pub struct OpenaiProvider {
    messages: VecDeque<Message>,
    model: String,
    token: String,
}

impl OpenaiProvider {
    /// defines the capacity of messages stored
    /// the the provider queue, number must be an even number
    const MAX_QUEUE_MESSAGES: usize = 8;

    fn new(model: String, token: String) -> OpenaiProvider {
        OpenaiProvider {
            messages: VecDeque::with_capacity(Self::MAX_QUEUE_MESSAGES),
            model,
            token,
        }
    }

    /// push given message to the queue with respect to the `MAX_QUEUE_MESSAGES`
    fn push_message(&mut self, message: Message) {
        if self.messages.len() > Self::MAX_QUEUE_MESSAGES {
            // pop the first 2 messages, the first message (expected) to always be user message
            // and since the second message is the agent response, we need to pop it because
            // openai chat cannot start with agent response
            self.messages.pop_front();
            self.messages.pop_front();
        }
        self.messages.push_back(message);
    }

    /// builds the request payload to send to openai, assuming the latest
    /// message is already pushed into the inner messages buffer
    fn preper_payload(&self) -> serde_json::Value {
        let messages = serde_json::to_value(&self.messages).unwrap();
        println!("messages {}", messages);
        json!({
            "model": self.model,
            "messages": messages,
            "stream": true
        })
    }

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

impl AgentProvider for OpenaiProvider {
    fn display_name(&self) -> String {
        format!("openai-{}", self.model)
    }

    fn request(
        &mut self,
        message: &str,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<AgentEvent>>>>> {
        self.push_message(Message::new(MessageRole::User, message.into()));
        let request = reqwest::Client::builder()
            .build()?
            .post("https://api.openai.com/v1/chat/completions")
            .bearer_auth(&self.token)
            .json(&self.preper_payload())
            .eventsource()?
            .map_ok(Self::parse_event)
            .map_err(|e| e.into());
        Ok(Box::pin(request))
    }
}

impl TryFrom<&Config> for OpenaiProvider {
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
        Ok(OpenaiProvider::new(model, token))
    }
}
