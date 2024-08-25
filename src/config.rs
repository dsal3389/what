use std::{env, path::PathBuf};

use anyhow::Context;
use serde::Deserialize;
use tokio::fs::{read_to_string, File};

#[derive(Deserialize)]
pub struct Configuration {
    pub token: String,
}

impl Configuration {
    const DEFAULT_CONFIG_NAME: &'static str = ".what.config.json";

    pub async fn parse(path_hint: Option<PathBuf>) -> anyhow::Result<Self> {
        let path = Self::resolve_config_path(path_hint)?;
        if !path.exists() {
            File::create(&path)
                .await
                .with_context(|| format!("couldn't create config file at {:?}", path))?;
            anyhow::bail!("couldn't find configuration file at {path:?}");
        }

        let content = read_to_string(&path).await?;
        serde_json::from_str(&content).map_err(anyhow::Error::new)
    }

    /// returns the path to the config path
    fn resolve_config_path(path_hint: Option<PathBuf>) -> anyhow::Result<PathBuf> {
        if let Some(home_path) = path_hint {
            return Ok(home_path);
        }
        let home_path = env::var("HOME").context("`$HOME` env variable is not set.")?;
        Ok(PathBuf::from(&home_path).join(Self::DEFAULT_CONFIG_NAME))
    }
}
