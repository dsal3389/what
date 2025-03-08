use std::pin::Pin;

use anyhow::{Context, Result};
use futures::Stream;

use crate::config::ConfigAgentProvider;
use crate::Config;

mod openai;
use openai::OpenaiProvider;

/// AgentEvent represent
/// an event that was sent from the agent
pub enum AgentEvent {
    Open,
    Text(String),
    End,
}

pub struct AgentResponse<'resp> {
    stream: Pin<Box<dyn Stream<Item = Result<AgentEvent>> + 'resp>>,
}

impl<'resp> AgentResponse<'resp> {
    fn new(stream: Box<dyn Stream<Item = Result<AgentEvent>> + 'resp>) -> AgentResponse<'resp> {
        AgentResponse {
            stream: stream.into(),
        }
    }
}

impl Stream for AgentResponse<'_> {
    type Item = Result<AgentEvent>;
    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.get_mut().stream.as_mut().poll_next(cx)
    }
}

pub trait AgentProvider {
    /// returns the provider display name, the returned
    /// string is used to display to the user what agent response to him
    fn display_name(&self) -> String;

    /// perform the request to the agent, most agents are capable
    /// of streaming the response, thus the response is async iter (Stream)
    fn request(&mut self, message: &str) -> Result<AgentResponse>;
}

/// a type that contains inside of it the provider
/// this type provides level of abstraction for easy provider
/// replacement, it is expected from the Agent to live for the whole
/// program life, thus the 'static lifetime
pub struct Agent {
    inner: Box<dyn AgentProvider>,
}

impl Agent {
    /// returns mutable reference to the inner agent provider
    pub fn get_provider(&mut self) -> &mut dyn AgentProvider {
        self.inner.as_mut()
    }
}

impl TryFrom<&Config> for Agent {
    type Error = anyhow::Error;
    fn try_from(value: &Config) -> std::result::Result<Self, Self::Error> {
        let provider: Box<dyn AgentProvider> = match value
            .provider
            .as_ref()
            .context("no agent provider was given")?
        {
            ConfigAgentProvider::Openai => Box::new(OpenaiProvider::try_from(value)?),
        };
        Ok(Agent { inner: provider })
    }
}
