use anyhow::{Context, Result};
use futures::{Stream, TryStreamExt};
use reqwest_eventsource::{Event, EventSource};

/// AgentEvent represent
/// an event that was sent from the agent
pub enum AgentEvent {
    Open,
    Text(String),
    End,
}

/// event source parser should be implemented on types
/// that can accept server event source and parse it to
/// a standard `AgentEvent`
pub trait AgentEventParser {
    /// takes agent specific event and parse it
    /// to a standard event interface
    fn parse_event(&self, event: Event) -> AgentEvent;
}

pub trait AgentRequester {
    /// returns the agent parser to parse
    /// incoming events triggered by the eventsource
    fn event_parser(&self) -> impl AgentEventParser;

    /// each agent have different request schemas and endpoints
    /// thus requiring each agent to build it own eventsource
    /// request that can be used
    fn build_eventsource(&self, message: &str) -> Result<EventSource>;

    /// performs the request to the agent with the given message
    /// and returns a stream which triggered everytime
    /// agent send event
    fn request(&self, message: &str) -> Result<impl Stream<Item = Result<AgentEvent>>> {
        let parser = self.event_parser();
        Ok(self
            .build_eventsource(message)
            .context("couldn't build eventsource")?
            .map_ok(move |e| parser.parse_event(e))
            .map_err(|e| e.into()))
    }
}

mod openai;
pub use openai::OpenAIAgent;
