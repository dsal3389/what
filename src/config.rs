use std::convert::AsRef;
use std::env;
use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const CONFIG_FILE_NAME: &str = ".what.conf.json";

#[derive(Deserialize, Serialize, Default, Debug)]
pub struct Config {
    pub openai_api_token: Option<String>,
    pub provider: Option<ConfigAgentProvider>,
    pub model: Option<String>,
}

#[derive(Deserialize, Serialize, Debug)]
pub enum ConfigAgentProvider {
    Openai,
}

impl Config {
    pub fn from_path<P>(p: P) -> Result<Config>
    where
        P: AsRef<Path>,
    {
        let reader = File::open(&p)
            .with_context(|| format!("couldn't open config path at {}", p.as_ref().display()))?;
        serde_json::from_reader(reader)
            .with_context(|| format!("couldn't parse config file at {}", p.as_ref().display()))
    }

    pub fn save<P>(&self, p: P) -> Result<()>
    where
        P: AsRef<Path>,
    {
        let file = File::create(p.as_ref())?;
        serde_json::to_writer_pretty(file, self)?;
        Ok(())
    }
}

pub fn default_config_path() -> PathBuf {
    let path: PathBuf = env::var("HOME")
        .expect("couldn't read environment variable `HOME`")
        .into();
    path.join(".config").join(CONFIG_FILE_NAME)
}
