//! Observation does not record. Only a validated, explicit action can authorize a start.
pub mod native;
pub mod session;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallState {
    ConfirmedCall,
    NoCall,
    Unknown,
    Unsupported,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Observation {
    pub application: String,
    pub process_id: Option<u32>,
    /// Includes OS process creation time. A reused PID cannot retain the old session.
    pub process_identity: Option<String>,
    pub state: CallState,
    pub evidence: Vec<String>,
    pub confidence: String,
    pub limitations: Vec<String>,
    pub observed_at_ms: u64,
}
impl Observation {
    pub fn unknown(application: &str, at: u64, limitation: &str) -> Self {
        Self {
            application: application.into(),
            process_id: None,
            process_identity: None,
            state: CallState::Unknown,
            evidence: vec![],
            confidence: "none".into(),
            limitations: vec![limitation.into()],
            observed_at_ms: at,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    pub applications: Vec<String>,
    pub debounce_ms: u64,
    pub grace_ms: u64,
    pub auto_stop: bool,
    pub notification_mode: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            applications: vec!["zoom".into(), "discord".into()],
            debounce_ms: 3_000,
            grace_ms: 20_000,
            auto_stop: false,
            notification_mode: "system".into(),
        }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        if !(500..=60_000).contains(&self.debounce_ms) {
            return Err("debounce_ms must be between 500 and 60000".into());
        }
        if !(5_000..=300_000).contains(&self.grace_ms) {
            return Err("grace_ms must be between 5000 and 300000".into());
        }
        if !["system", "in_app"].contains(&self.notification_mode.as_str()) {
            return Err("notification_mode must be system or in_app".into());
        }
        if self
            .applications
            .iter()
            .any(|a| !["zoom", "discord"].contains(&a.as_str()))
        {
            return Err("Only Zoom and Discord desktop adapters are currently supported".into());
        }
        Ok(())
    }
}

/// Implementations execute on one dedicated native observer thread (COM MTA on Windows).
pub trait Observer {
    fn observe(&mut self, at_ms: u64) -> Vec<Observation>;
}

/// UI adapters read only call-control names, never chat bodies or window document text.
pub fn call_control_matches(
    application: &str,
    button_names: &[String],
    status_names: &[String],
) -> bool {
    let names: Vec<String> = button_names
        .iter()
        .map(|s| s.trim().to_lowercase())
        .collect();
    match application {
        "zoom" => {
            names.iter().any(|s| {
                [
                    "leave meeting",
                    "end meeting",
                    "покинуть конференцию",
                    "завершить конференцию",
                    "выйти из конференции",
                ]
                .contains(&s.as_str())
            }) || (names
                .iter()
                .any(|s| ["leave", "end", "выйти", "завершить"].contains(&s.as_str()))
                && names
                    .iter()
                    .any(|s| s.starts_with("participants") || s.starts_with("участники")))
        }
        "discord" => {
            names
                .iter()
                .any(|s| ["disconnect", "отключиться", "отключить"].contains(&s.as_str()))
                && status_names.iter().any(|s| {
                    let s = s.to_lowercase();
                    s.contains("voice connected")
                        || s.contains("rtc connected")
                        || s.contains("голосовая связь установлена")
                        || s.contains("подключено к голосовому каналу")
                })
        }
        _ => false,
    }
}
