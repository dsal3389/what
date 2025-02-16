use std::fmt::Display;
use std::pin::Pin;

use anyhow::{Context, Result};
use clap::ValueEnum;
use futures::Stream;
use serde::Deserialize;

use crate::Config;

#[derive(Debug, Clone, ValueEnum, Deserialize)]
pub enum AgentProvider {
    Openai,
}

/// AgentEvent represent
/// an event that was sent from the agent
pub enum AgentEvent {
    Open,
    Text(String),
    End,
}

pub trait AgentClient: Display {
    fn request(&self, message: &str) -> Result<Pin<Box<dyn Stream<Item = Result<AgentEvent>>>>>;
}

pub struct Agent {
    inner: Box<dyn AgentClient>,
}

impl AgentClient for Agent {
    fn request(&self, message: &str) -> Result<Pin<Box<dyn Stream<Item = Result<AgentEvent>>>>> {
        self.inner.request(message)
    }
}

impl Display for Agent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.inner)
    }
}

impl TryFrom<&Config> for Agent {
    type Error = anyhow::Error;
    fn try_from(value: &Config) -> std::result::Result<Self, Self::Error> {
        let provider = match value
            .provider
            .as_ref()
            .context("couldn't find provider value")?
        {
            AgentProvider::Openai => OpenAIProvider::try_from(value),
        }?;
        Ok(Agent {
            inner: Box::new(provider),
        })
    }
}

mod openai;
pub use openai::OpenAIProvider;
