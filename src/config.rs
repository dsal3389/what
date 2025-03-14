use std::convert::AsRef;
use std::env;
use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::de::Visitor;
use serde::{Deserialize, Serialize};

const CONFIG_FILE_NAME: &str = ".what.conf.json";

#[derive(Deserialize, Serialize, Default, Debug)]
pub struct Config {
    pub openai_api_token: Option<String>,
    pub provider: Option<ConfigAgentProvider>,
    pub model: Option<String>,
}

#[derive(Debug)]
pub enum ConfigAgentProvider {
    Openai,
}

impl<'de> Deserialize<'de> for ConfigAgentProvider {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_str(ConfigAgentProviderVisitor)
    }
}

impl Serialize for ConfigAgentProvider {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Openai => serializer.serialize_unit_variant("ConfigAgentprovider", 0, "openai"),
        }
    }
}

impl TryFrom<&str> for ConfigAgentProvider {
    type Error = anyhow::Error;
    fn try_from(value: &str) -> std::result::Result<Self, Self::Error> {
        match value {
            "openai" => Ok(Self::Openai),
            _ => anyhow::bail!(
                "couldn't convert string `{}` to a valid config agent provider",
                value
            ),
        }
    }
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

struct ConfigAgentProviderVisitor;

impl<'de> Visitor<'de> for ConfigAgentProviderVisitor {
    type Value = ConfigAgentProvider;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("expected string name of provider")
    }

    fn visit_string<E>(self, v: String) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.visit_str(&v)
    }

    fn visit_str<E>(self, v: &str) -> std::result::Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        match v.to_lowercase().as_str() {
            "openai" => Ok(ConfigAgentProvider::Openai),
            p => Err(E::custom(format!("unknown provider {}", p))),
        }
    }
}
