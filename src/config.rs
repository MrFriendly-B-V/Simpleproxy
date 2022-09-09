use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub net: NetConfig,
    pub tls: Option<TlsConfig>,
    pub routes: Vec<Route>,
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
pub struct Route {
    pub path_prefix: Option<String>,
    pub host: Option<String>,
    pub default: Option<bool>,
    pub upstream: String,
    // TODO support authentication
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
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),
}

impl Default for Config {
    fn default() -> Self {
        Self {
            net: NetConfig::default(),
            tls: Some(TlsConfig::default()),
            routes: vec![
                Route::default(),
            ]
        }
    }
}

impl Default for NetConfig {
    fn default() -> Self {
        Self {
            port: 8080,
            bind_address: "0.0.0.0".into(),
        }
    }
}

impl Default for Route {
    fn default() -> Self {
        Self {
            host: Some("foo.example.com".into()),
            path_prefix: Some("/bar".into()),
            upstream: "http://foo-bar.internal.example.com:8080".into(),
            default: Some(false),
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

        if self.routes.iter().filter(|x| x.default.eq(&Some(true))).count() > 1 {
            return Err(ConfigError::InvalidConfig("Only one default route is allowed".into()))
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
