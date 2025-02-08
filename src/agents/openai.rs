use std::env;

use anyhow::{Context, Result};
use reqwest_eventsource::{Event, EventSource, RequestBuilderExt};
use serde_json::json;

use super::{AgentEvent, AgentEventParser, AgentRequester};

struct OpenAIEventParser;

impl AgentEventParser for OpenAIEventParser {
    fn parse_event(&self, event: Event) -> AgentEvent {
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

#[derive(Debug)]
pub struct OpenAIAgent;

impl AgentRequester for OpenAIAgent {
    fn event_parser(&self) -> impl AgentEventParser {
        OpenAIEventParser
    }

    fn build_eventsource(&self, message: &str) -> Result<EventSource> {
        let token = env::var("OPENAI_TOKEN").context("coudln't find OpenAI API token")?;
        Ok(reqwest::Client::builder()
            .build()?
            .post("https://api.openai.com/v1/chat/completions")
            .bearer_auth(token)
            .json(&json!({
                "model": "gpt-3.5-turbo",
                "stream": true,
                "messages": [{
                    "role": "user",
                    "content": message
                }]
            }))
            .eventsource()
            .unwrap())
    }
}
