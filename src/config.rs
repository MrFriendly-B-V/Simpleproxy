use std::collections::HashMap;
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
    pub proxy: Option<ProxyConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProxyConfig {
    /// The value for the `Server` header
    /// to be set on responses in case of an error.
    /// This makes it easy to track if the error
    /// occurred at the proxy (e.g. a misconfigured route)
    pub error_server_header: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetConfig {
    pub port: u16,
    pub bind_address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsConfig {
    pub pubkey: PathBuf,
    pub privkey: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Route {
    /// The path prefix for this route to match on.
    /// E.g. setting this to `/foo` will route `/foo/bar`, `/foo/foo/bar` to this route,
    /// but `/bar/foo` will not be routed to this route.
    pub path_prefix: Option<String>,
    /// The host this route matches on. E.g. `foo.example.com`
    pub host: Option<String>,
    /// Whether this should be the default route (i.e. fallback)
    /// if no other route matches. Only one route may have this
    /// set to true per host. For routes without `host` specified,
    /// there may only be 1 default route.
    pub default: Option<bool>,
    /// The upstream server
    /// This includes the protocol, e.g. `https://`
    pub upstream: String,
    /// Whether the `path_prefix` should be stripped from the request path
    /// E.g. if the `path_prefix` is `/foo`, and the request path is `/foo/bar`,
    /// with this option enabled the path becomes just `/bar`
    pub strip_path_prefix: Option<bool>,
    // TODO support authorization
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
            ],
            proxy: None,
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

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            privkey: PathBuf::from("/etc/your/priv/key.pem"),
            pubkey: PathBuf::from("/etc/your/pub/key.pem"),
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
            strip_path_prefix: Some(false),
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


        // Check the `default` parameter
        let mut host_default_count: HashMap<&str, usize> = HashMap::with_capacity(self.routes.len());
        let mut no_host_default_count = 0_usize;
        for route in &self.routes {
            if let Some(host) = &route.host {
                host_default_count.entry(&**host)
                    .and_modify(|x| *x += 1)
                    .or_insert(0);
            } else {
                no_host_default_count += 1;
            }
        }

        for (host, default_count) in host_default_count {
            if default_count > 1 {
                return Err(ConfigError::InvalidConfig(format!("The host {host} may only have 1 default route. It currently has {default_count}")));
            }
        }

        if no_host_default_count > 1 {
            return Err(ConfigError::InvalidConfig(format!("There may only be 1 default route without a specified host. There are currently {no_host_default_count}")));
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
