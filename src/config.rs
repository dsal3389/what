use std::convert::AsRef;
use std::env;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::agents::AgentProvider;

const CONFIG_FILE_NAME: &str = ".what.conf.json";

#[derive(Default, Deserialize, Debug)]
pub struct Config {
    pub openai_api_token: Option<String>,
    pub provider: Option<AgentProvider>,
    pub model: Option<String>,
}

impl Config {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Config> {
        let buffer = File::open(&path)
            .with_context(|| format!("couldn't open config path at {}", path.as_ref().display()))
            .map(|file| BufReader::new(file))?;
        serde_json::from_reader(buffer)
            .with_context(|| format!("couldn't parse config file at {}", path.as_ref().display()))
    }
}

pub fn default_config_path() -> Result<PathBuf> {
    env::var("HOME")
        .context("couldn't get user home dir")
        .map(|v| PathBuf::from(v).join(".config").join(CONFIG_FILE_NAME))
}
