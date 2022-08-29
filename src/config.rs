use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    pub net: NetConfig,
    pub tls: Option<TlsConfig>,
    pub proxy: ProxyConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetConfig {
    pub port: u16,
    pub bind_address: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TlsConfig {
    pub pubkey: PathBuf,
    pub privkey: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    /// The routes to proxy to
    /// K = The prefix of the path
    /// V = The upstream server to route to
    pub prefix_routes: HashMap<String, String>,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("Failed to serialize: {0}")]
    TomlSer(#[from] toml::ser::Error),
    #[error("Failed to deserialize: {0}")]
    TomlDe(#[from] toml::de::Error),
    #[error("Configured path does not exist: {0}")]
    FileNotFound(PathBuf),
}

impl Default for NetConfig {
    fn default() -> Self {
        Self {
            port: 8080,
            bind_address: "0.0.0.0".into(),
        }
    }
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            prefix_routes: vec![("/foo".into(), "http://foo.example.com".into())].into_iter().collect::<HashMap<_, _>>(),
        }
    }
}

impl Config {
    pub async fn new<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        if !path.exists() {
            return Self::create_default(path).await;
        }

        let mut f = fs::File::open(path).await?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).await?;

        let deserialized: Self = toml::de::from_slice(&buf)?;
        deserialized.validate()?;

        Ok(deserialized)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if let Some(tls) = &self.tls {
            if !tls.pubkey.exists() {
                return Err(ConfigError::FileNotFound(tls.pubkey.clone()));
            }

            if !tls.privkey.exists() {
                return Err(ConfigError::FileNotFound(tls.privkey.clone()));
            }
        }

        Ok(())
    }

    async fn create_default(path: &Path) -> Result<Self, ConfigError> {
        let this = Self::default();
        let serialized = toml::ser::to_string_pretty(&this)?;

        if let Some(parent_dir) = path.parent() {
            fs::create_dir_all(parent_dir).await?;
        }

        let mut f = fs::File::create(path).await?;
        f.write_all(serialized.as_bytes()).await?;

        Ok(this)
    }
}
