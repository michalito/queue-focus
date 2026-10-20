use qf_core::sync::*;
use reqwest::{blocking::Client, redirect::Policy, Url};
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    io::{self, Read},
    net::IpAddr,
    path::Path,
    time::Duration,
};

pub const CONFIG_FILE: &str = "sync-config.json";
const CONFIG_LIMIT: usize = 4096;

// Intentionally no derived Debug: the credential must never appear in a log.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub url: String,
    pub device: Id,
    pub token: String,
    pub allow_loopback_http: bool,
}
impl Config {
    pub fn load(directory: &Path) -> io::Result<Self> {
        let value: Self = read_private(&directory.join(CONFIG_FILE), CONFIG_LIMIT)?;
        value.endpoint().map_err(io::Error::other)?;
        Ok(value)
    }
    pub fn save(&self, directory: &Path) -> Result<(), qf_core::SaveError> {
        self.endpoint()
            .map_err(|e| qf_core::SaveError::BeforeCommit(io::Error::other(e)))?;
        save_private(&directory.join(CONFIG_FILE), self, CONFIG_LIMIT)
    }
    pub fn endpoint(&self) -> Result<Url, Failure> {
        if self.version != PROTOCOL_VERSION
            || self.device.is_nil()
            || self.token.is_empty()
            || self.token.len() > 256
            || !self.token.bytes().all(|b| b.is_ascii_alphanumeric())
        {
            return Err(Failure::Configuration(
                "invalid sync credentials or configuration version".into(),
            ));
        }
        let mut url = Url::parse(&self.url)
            .map_err(|_| Failure::Configuration("invalid server URL".into()))?;
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
            || url.host().is_none()
        {
            return Err(Failure::Configuration(
                "use the server origin URL without credentials, path, query or fragment".into(),
            ));
        }
        let loopback = url
            .host_str()
            .and_then(|s| {
                s.trim_start_matches('[')
                    .trim_end_matches(']')
                    .parse::<IpAddr>()
                    .ok()
            })
            .is_some_and(|ip| ip.is_loopback());
        if url.scheme() != "https"
            && !(url.scheme() == "http" && self.allow_loopback_http && loopback)
        {
            return Err(Failure::Configuration(
                "sync requires HTTPS (explicit loopback HTTP is available for local tests)".into(),
            ));
        }
        url.set_path("/v1/exchange");
        Ok(url)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    Network,
    Http {
        status: u16,
        reason: Option<Rejection>,
    },
    InvalidResponse,
    Configuration(String),
    Clock,
}
impl Failure {
    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Self::Network
                | Self::Clock
                | Self::Http {
                    status: 408 | 429 | 500..=599,
                    ..
                }
        )
    }
    pub fn recovery_required(&self) -> bool {
        matches!(
            self,
            Self::Http {
                status: 409,
                reason: Some(Rejection::WrongDataset | Rejection::HistoryMismatch)
            }
        )
    }
}
impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Network => f.write_str("Sync is offline; saved changes will retry."),
            Self::Http {
                status: 401 | 403, ..
            } => {
                f.write_str("Sync credentials need attention. Pending commands have been retained.")
            }
            Self::Http {
                status: 409,
                reason: Some(Rejection::WrongDataset | Rejection::HistoryMismatch),
            } => f.write_str(
                "Server history changed. Export this queue into a new directory to recover.",
            ),
            Self::Http { status, .. } => write!(
                f,
                "Sync server returned HTTP {status}. Pending commands have been retained."
            ),
            Self::InvalidResponse => f.write_str(
                "Sync server returned an invalid response. Pending commands have been retained.",
            ),
            Self::Configuration(message) => f.write_str(message),
            Self::Clock => f.write_str("The boot clock is unavailable; sync will retry."),
        }
    }
}
impl std::error::Error for Failure {}

pub struct Transport {
    client: Client,
    config: Config,
    endpoint: Url,
}
impl Transport {
    /// Construct on the worker thread or from the setup CLI.
    pub fn new(config: Config) -> Result<Self, Failure> {
        let endpoint = config.endpoint()?;
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(3))
            .redirect(Policy::none())
            .no_proxy()
            .user_agent(concat!("QueueFocus/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| Failure::Configuration("could not initialize HTTPS transport".into()))?;
        Ok(Self {
            client,
            config,
            endpoint,
        })
    }
    pub fn exchange(&self, request: &ExchangeRequest) -> Result<ExchangeResponse, Failure> {
        request
            .validate()
            .map_err(|_| Failure::Configuration("invalid exchange request".into()))?;
        let bytes = serde_json::to_vec(request)
            .map_err(|_| Failure::Configuration("cannot encode exchange request".into()))?;
        if bytes.len() > MAX_HTTP_BYTES {
            return Err(Failure::Configuration(
                "exchange request exceeds size limit".into(),
            ));
        }
        let response = self
            .client
            .post(self.endpoint.clone())
            .bearer_auth(&self.config.token)
            .header("x-queue-focus-device", self.config.device.to_string())
            .header("content-type", "application/json")
            .body(bytes)
            .send()
            .map_err(|_| Failure::Network)?;
        let status = response.status().as_u16();
        if response
            .content_length()
            .is_some_and(|n| n > MAX_HTTP_BYTES as u64)
        {
            return Err(Failure::InvalidResponse);
        }
        let mut bytes = Vec::new();
        response
            .take(MAX_HTTP_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Failure::Network)?;
        if bytes.len() > MAX_HTTP_BYTES {
            return Err(Failure::InvalidResponse);
        }
        if status != 200 {
            #[derive(Deserialize)]
            struct ErrorBody {
                reason: Option<Rejection>,
            }
            let reason = serde_json::from_slice::<ErrorBody>(&bytes)
                .ok()
                .and_then(|body| body.reason);
            return Err(Failure::Http { status, reason });
        }
        let response: ExchangeResponse =
            serde_json::from_slice(&bytes).map_err(|_| Failure::InvalidResponse)?;
        if response.version != PROTOCOL_VERSION
            || response.device != self.config.device
            || response.snapshot.validate().is_err()
        {
            return Err(Failure::InvalidResponse);
        }
        Ok(response)
    }
}
