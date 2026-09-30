use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

pub const APP_ID: &str = "com.youngteurus.meetily.calls";
#[derive(Clone, Deserialize)]
pub struct Credentials {
    pub port: u16,
    pub read_token: String,
    pub control_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub code: String,
    pub message: String,
}
impl RpcError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new("internal", message)
    }
    pub fn exit_code(&self) -> i32 {
        match self.code.as_str() {
            "invalid_request" | "invalid_language" | "unsupported_language" => 2,
            "unavailable"
            | "not_ready"
            | "recording_start_failed"
            | "recording_transition_failed" => 3,
            "unauthorized" | "forbidden" => 4,
            "not_found" | "no_recording" | "unknown_method" => 5,
            "conflict" | "recording_active" | "recording_mismatch" => 6,
            "timeout" => 7,
            _ => 1,
        }
    }
}
impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for RpcError {}

pub fn discovery_path() -> Result<PathBuf, RpcError> {
    if let Some(path) = std::env::var_os("MEETILY_INTEGRATION_FILE") {
        return Ok(PathBuf::from(path));
    }
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base =
        std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Library/Application Support"));
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share")));
    base.map(|p| p.join(APP_ID).join("integration.json"))
        .ok_or_else(|| {
            RpcError::new(
                "unavailable",
                "Cannot locate app data; set MEETILY_INTEGRATION_FILE",
            )
        })
}

#[derive(Clone)]
pub struct Gateway {
    client: reqwest::Client,
    credentials: Credentials,
    discovery_file: Option<PathBuf>,
}
impl Gateway {
    pub fn discover() -> Result<Self, RpcError> {
        Self::from_file(&discovery_path()?)
    }
    pub fn from_file(path: &Path) -> Result<Self, RpcError> {
        let credentials = load_credentials(path)?;
        let mut gateway = Self::new(credentials)?;
        gateway.discovery_file = Some(path.to_owned());
        Ok(gateway)
    }
    pub fn new(credentials: Credentials) -> Result<Self, RpcError> {
        if credentials.port == 0 || credentials.read_token.is_empty() {
            return Err(RpcError::new(
                "invalid_request",
                "Integration port and read credential must be present",
            ));
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|_| RpcError::internal("Could not initialize gateway client"))?;
        Ok(Self {
            client,
            credentials,
            discovery_file: None,
        })
    }
    pub fn has_control(&self) -> bool {
        self.credentials
            .control_token
            .as_ref()
            .is_some_and(|s| !s.is_empty())
    }
    pub async fn call(
        &self,
        method: &str,
        mut params: Value,
        control: bool,
    ) -> Result<Value, RpcError> {
        // Follow an app restart/credential rotation without caching its old
        // ephemeral port. Explicit new() clients intentionally remain static.
        let refreshed;
        let credentials = if let Some(path) = &self.discovery_file {
            refreshed = load_credentials(path)?;
            &refreshed
        } else {
            &self.credentials
        };
        let token = if control {
            credentials.control_token.as_ref().filter(|s| !s.is_empty()).ok_or_else(|| RpcError::new("forbidden","Recording control is not enabled; enable the separate control permission in Meetily"))?
        } else {
            &credentials.read_token
        };
        // Optional values are omitted, so backend defaults retain their meaning.
        if let Some(object) = params.as_object_mut() {
            object.retain(|_, v| !v.is_null());
        }
        let response = self.client.post(format!("http://127.0.0.1:{}/v1/rpc",credentials.port)).bearer_auth(token).json(&json!({"method":method,"params":params})).send().await.map_err(|e| {
            if e.is_timeout() { RpcError::new("timeout","Meetily gateway request timed out") }
            else { RpcError::new("unavailable","Cannot connect to local Meetily. Start the app and check integrations are enabled") }
        })?;
        let status = response.status();
        let envelope: Value = response
            .json()
            .await
            .map_err(|_| RpcError::internal("Gateway returned invalid JSON"))?;
        if let Some(error) = envelope.get("error") {
            return Err(serde_json::from_value(error.clone())
                .unwrap_or_else(|_| RpcError::internal("Gateway returned an invalid error")));
        }
        if !status.is_success() {
            return Err(RpcError::new(
                match status.as_u16() {
                    401 => "unauthorized",
                    403 => "forbidden",
                    _ => "unavailable",
                },
                format!("Gateway returned HTTP {status}"),
            ));
        }
        envelope
            .get("result")
            .cloned()
            .ok_or_else(|| RpcError::internal("Gateway response lacks result"))
    }
}

fn load_credentials(path: &Path) -> Result<Credentials, RpcError> {
    let bytes = std::fs::read(path).map_err(|_| {
        RpcError::new(
            "unavailable",
            format!(
                "Cannot read {}. Start Meetily Calls and enable local integrations",
                path.display()
            ),
        )
    })?;
    let credentials: Credentials = serde_json::from_slice(&bytes).map_err(|_| {
        RpcError::new(
            "invalid_request",
            "Invalid integration credential file; regenerate it in Meetily settings",
        )
    })?;
    if credentials.port == 0 || credentials.read_token.is_empty() {
        return Err(RpcError::new(
            "invalid_request",
            "Integration port and read credential must be present",
        ));
    }
    Ok(credentials)
}
