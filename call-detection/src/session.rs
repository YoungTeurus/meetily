use crate::{CallState, Observation, Settings};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Candidate,
    InCall,
    Uncertain,
    Leaving,
    Ended,
}

#[derive(Clone, Debug, Serialize)]
pub struct Session {
    pub session_id: String,
    pub application: String,
    pub phase: Phase,
    pub suppressed: bool,
    pub recording_id: Option<String>,
    pub starting: bool,
    #[serde(skip)]
    identity: String,
    #[serde(skip)]
    positive_since: u64,
    #[serde(skip)]
    absent_since: Option<u64>,
    #[serde(skip)]
    last_state: CallState,
    #[serde(skip)]
    offered: bool,
    #[serde(skip)]
    stop_offered: bool,
    #[serde(skip)]
    stop_suppressed: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Prompt {
    pub kind: String,
    pub session_id: String,
    pub application: String,
}
#[derive(Default)]
pub struct Engine {
    sessions: BTreeMap<String, Session>,
    generation: u64,
}
impl Engine {
    pub fn sessions(&self) -> Vec<Session> {
        self.sessions.values().cloned().collect()
    }
    pub fn prompts(&self) -> Vec<Prompt> {
        self.sessions
            .values()
            .filter_map(|s| {
                let kind = if s.phase == Phase::InCall
                    && s.offered
                    && !s.suppressed
                    && s.recording_id.is_none()
                    && !s.starting
                {
                    "start"
                } else if s.phase == Phase::Ended
                    && s.last_state == CallState::NoCall
                    && s.stop_offered
                    && s.recording_id.is_some()
                {
                    "stop"
                } else {
                    return None;
                };
                Some(Prompt {
                    kind: kind.into(),
                    session_id: s.session_id.clone(),
                    application: s.application.clone(),
                })
            })
            .collect()
    }
    pub fn update(
        &mut self,
        observations: &[Observation],
        settings: &Settings,
        now: u64,
    ) -> Vec<Prompt> {
        let mut fresh = vec![];
        if !settings.enabled {
            self.disable();
            return fresh;
        }
        for s in self.sessions.values_mut() {
            if !settings.applications.contains(&s.application) {
                s.suppressed = true;
                s.offered = false;
                s.stop_offered = false;
            }
        }
        for o in observations {
            if !settings.enabled || !settings.applications.contains(&o.application) {
                continue;
            }
            let identity = o
                .process_identity
                .clone()
                .unwrap_or_else(|| o.application.clone());
            let replace = self
                .sessions
                .get(&o.application)
                .map(|s| {
                    !s.starting
                        && ((s.phase == Phase::Ended && s.recording_id.is_none())
                            || (s.identity != identity
                                && o.state == CallState::ConfirmedCall
                                && s.recording_id.is_none()))
                })
                .unwrap_or(true);
            if replace && o.state == CallState::ConfirmedCall {
                self.generation += 1;
                self.sessions.insert(
                    o.application.clone(),
                    Session {
                        session_id: format!("{}-{now}-{}", o.application, self.generation),
                        application: o.application.clone(),
                        phase: Phase::Candidate,
                        suppressed: false,
                        recording_id: None,
                        starting: false,
                        identity,
                        positive_since: now,
                        absent_since: None,
                        last_state: o.state,
                        offered: false,
                        stop_offered: false,
                        stop_suppressed: false,
                    },
                );
            }
            let Some(s) = self.sessions.get_mut(&o.application) else {
                continue;
            };
            let previous_state = s.last_state;
            s.last_state = o.state;
            match o.state {
                CallState::ConfirmedCall => {
                    s.absent_since = None;
                    if s.phase == Phase::Ended {
                        if s.recording_id.is_some() {
                            s.phase = Phase::InCall;
                            s.stop_offered = false;
                            s.stop_suppressed = false;
                        } else {
                            continue;
                        }
                    }
                    if s.phase == Phase::Candidate {
                        if previous_state != CallState::ConfirmedCall {
                            s.positive_since = now;
                        }
                        if now.saturating_sub(s.positive_since) >= settings.debounce_ms {
                            s.phase = Phase::InCall;
                        }
                    } else {
                        s.phase = Phase::InCall;
                    }
                    if s.phase == Phase::InCall
                        && !s.suppressed
                        && !s.offered
                        && s.recording_id.is_none()
                        && !s.starting
                    {
                        s.offered = true;
                        fresh.push(Prompt {
                            kind: "start".into(),
                            session_id: s.session_id.clone(),
                            application: s.application.clone(),
                        });
                    }
                }
                CallState::NoCall => {
                    if s.phase == Phase::Candidate {
                        s.positive_since = now;
                        s.phase = Phase::Ended;
                    } else if s.phase == Phase::Ended
                        && s.recording_id.is_some()
                        && !s.stop_offered
                        && !s.stop_suppressed
                    {
                        s.stop_offered = true;
                        fresh.push(Prompt {
                            kind: "stop".into(),
                            session_id: s.session_id.clone(),
                            application: s.application.clone(),
                        });
                    } else if s.phase != Phase::Ended {
                        let since = *s.absent_since.get_or_insert(now);
                        s.phase = Phase::Leaving;
                        if now.saturating_sub(since) >= settings.grace_ms {
                            s.phase = Phase::Ended;
                            if s.recording_id.is_some() && !s.stop_offered && !s.stop_suppressed {
                                s.stop_offered = true;
                                fresh.push(Prompt {
                                    kind: "stop".into(),
                                    session_id: s.session_id.clone(),
                                    application: s.application.clone(),
                                });
                            }
                        }
                    }
                }
                CallState::Unknown | CallState::Unsupported => {
                    // Missing permissions/UI data are not evidence of leaving a meeting.
                    s.absent_since = None;
                    if s.phase == Phase::Candidate {
                        s.positive_since = now;
                    } else if s.phase != Phase::Ended {
                        s.phase = Phase::Uncertain;
                    }
                }
            }
        }
        fresh
    }
    fn find_mut(&mut self, id: &str) -> Result<&mut Session, String> {
        self.sessions
            .values_mut()
            .find(|s| s.session_id == id)
            .ok_or_else(|| "stale_detection_session".into())
    }
    pub fn authorize_start(&mut self, id: &str) -> Result<String, String> {
        let s = self.find_mut(id)?;
        if s.phase != Phase::InCall
            || s.last_state != CallState::ConfirmedCall
            || s.suppressed
            || s.recording_id.is_some()
            || s.starting
        {
            return Err("stale_or_suppressed_detection_session".into());
        }
        Ok(s.application.clone())
    }
    pub fn attach_recording(&mut self, id: &str, recording_id: &str) -> Result<(), String> {
        let s = self.find_mut(id)?;
        s.recording_id = Some(recording_id.into());
        s.starting = false;
        s.suppressed = true;
        Ok(())
    }
    pub fn reserve_start(&mut self, id: &str) -> Result<(), String> {
        let s = self.find_mut(id)?;
        s.starting = true;
        Ok(())
    }
    pub fn validate_identity(&self, id: &str, identity: Option<&str>) -> Result<(), String> {
        let s = self
            .sessions
            .values()
            .find(|s| s.session_id == id)
            .ok_or("stale_detection_session")?;
        if identity != Some(s.identity.as_str()) {
            return Err("detection_process_identity_changed".into());
        }
        Ok(())
    }
    pub fn validate_reserved_start(&self, id: &str, identity: Option<&str>) -> Result<(), String> {
        self.validate_identity(id, identity)?;
        let s = self
            .sessions
            .values()
            .find(|s| s.session_id == id)
            .ok_or("stale_detection_session")?;
        if !s.starting || s.phase != Phase::InCall || s.last_state != CallState::ConfirmedCall {
            return Err("reserved_detection_session_no_longer_active".into());
        }
        Ok(())
    }
    pub fn start_failed(&mut self, id: &str) {
        if let Ok(s) = self.find_mut(id) {
            s.starting = false;
        }
    }
    pub fn suppress(&mut self, id: &str) -> Result<(), String> {
        let s = self.find_mut(id)?;
        s.suppressed = true;
        if s.phase == Phase::Ended && s.stop_offered {
            s.stop_suppressed = true;
        }
        s.stop_offered = false;
        Ok(())
    }
    /// Called after explicit manual/CLI stop or a status refresh. Never associates unrelated recordings.
    pub fn recording_finished(&mut self, recording_id: &str) {
        for s in self.sessions.values_mut() {
            if s.recording_id.as_deref() == Some(recording_id) {
                s.recording_id = None;
                s.suppressed = true;
                s.stop_offered = false;
            }
        }
    }
    pub fn authorize_stop(
        &mut self,
        id: &str,
        current_recording_id: Option<&str>,
    ) -> Result<String, String> {
        let s = self.find_mut(id)?;
        if s.phase != Phase::Ended || s.last_state != CallState::NoCall || !s.stop_offered {
            return Err("call_has_not_confirmed_ended".into());
        }
        match s.recording_id.as_deref() {
            Some(r) if Some(r) == current_recording_id => Ok(r.into()),
            _ => Err("recording_not_owned_by_detection_session".into()),
        }
    }
    pub fn disable(&mut self) {
        for s in self.sessions.values_mut() {
            s.suppressed = true;
            s.offered = false;
            s.stop_offered = false;
        }
    }
}
