use std::convert::AsRef;
use std::env;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

const CONFIG_FILE_NAME: &str = ".what.conf.json";

#[derive(Default, Deserialize, Debug)]
pub struct Config {
    pub openai_default_model: Option<String>,
    pub openai_api_token: Option<String>,
}

impl Config {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Config> {
        let file = File::open(&path)
            .with_context(|| format!("couldn't open config path at {}", path.as_ref().display()))?;
        let buffer = BufReader::new(file);
        Ok(serde_json::from_reader(buffer)?)
    }
}

pub fn default_config_path() -> Result<PathBuf> {
    env::var("HOME")
        .context("couldn't get user home dir")
        .map(|v| PathBuf::from(v).join(".config").join(CONFIG_FILE_NAME))
}
