use call_detection::{
    call_control_matches,
    session::{Engine, Phase},
    CallState, Observation, Settings,
};
fn settings() -> Settings {
    Settings {
        enabled: true,
        ..Settings::default()
    }
}
fn observation(state: CallState, at: u64) -> Observation {
    Observation {
        application: "zoom".into(),
        process_id: Some(10),
        process_identity: Some("10-creation1".into()),
        state,
        evidence: vec![],
        confidence: "high".into(),
        limitations: vec![],
        observed_at_ms: at,
    }
}
fn update(e: &mut Engine, s: CallState, t: u64) {
    e.update(&[observation(s, t)], &settings(), t);
}
#[test]
fn debounce_and_no_automatic_recording() {
    let mut e = Engine::default();
    update(&mut e, CallState::ConfirmedCall, 0);
    assert!(e.prompts().is_empty());
    update(&mut e, CallState::ConfirmedCall, 2999);
    assert!(e.prompts().is_empty());
    update(&mut e, CallState::ConfirmedCall, 3000);
    assert_eq!(e.prompts()[0].kind, "start");
    assert!(e.sessions()[0].recording_id.is_none());
}
#[test]
fn skip_and_dismiss_suppress_same_call() {
    for _action in ["skip", "dismiss"] {
        let mut e = Engine::default();
        update(&mut e, CallState::ConfirmedCall, 0);
        update(&mut e, CallState::ConfirmedCall, 3000);
        let id = e.sessions()[0].session_id.clone();
        e.suppress(&id).unwrap();
        update(&mut e, CallState::ConfirmedCall, 6000);
        assert!(e.prompts().is_empty());
        assert!(e.authorize_start(&id).is_err());
    }
}
#[test]
fn mute_device_change_and_reconnect_do_not_end() {
    let mut e = Engine::default();
    update(&mut e, CallState::ConfirmedCall, 0);
    update(&mut e, CallState::ConfirmedCall, 3000);
    let id = e.sessions()[0].session_id.clone();
    e.attach_recording(&id, "r1").unwrap();
    update(&mut e, CallState::NoCall, 4000);
    update(&mut e, CallState::ConfirmedCall, 23000);
    assert_eq!(e.sessions()[0].phase, Phase::InCall);
    assert!(e.prompts().is_empty());
    update(&mut e, CallState::Unknown, 100000);
    assert_eq!(e.sessions()[0].phase, Phase::Uncertain);
    assert!(e.prompts().is_empty());
}
#[test]
fn ended_only_after_twenty_seconds_and_cannot_stop_unrelated() {
    let mut e = Engine::default();
    update(&mut e, CallState::ConfirmedCall, 0);
    update(&mut e, CallState::ConfirmedCall, 3000);
    let id = e.sessions()[0].session_id.clone();
    e.attach_recording(&id, "owned").unwrap();
    update(&mut e, CallState::NoCall, 4000);
    update(&mut e, CallState::NoCall, 23999);
    assert!(e.authorize_stop(&id, Some("owned")).is_err());
    update(&mut e, CallState::NoCall, 24000);
    assert_eq!(e.prompts()[0].kind, "stop");
    assert!(e.authorize_stop(&id, Some("manual")).is_err());
    assert_eq!(e.authorize_stop(&id, Some("owned")).unwrap(), "owned");
}
#[test]
fn stale_start_immediately_rejected_on_exit() {
    let mut e = Engine::default();
    update(&mut e, CallState::ConfirmedCall, 0);
    update(&mut e, CallState::ConfirmedCall, 3000);
    let id = e.sessions()[0].session_id.clone();
    update(&mut e, CallState::NoCall, 4000);
    assert!(e.authorize_start(&id).is_err());
    update(&mut e, CallState::NoCall, 24000);
    update(&mut e, CallState::ConfirmedCall, 25000);
    assert_ne!(id, e.sessions()[0].session_id);
    assert!(e.authorize_start(&id).is_err());
}
#[test]
fn manual_stop_suppresses_offer() {
    let mut e = Engine::default();
    update(&mut e, CallState::ConfirmedCall, 0);
    update(&mut e, CallState::ConfirmedCall, 3000);
    let id = e.sessions()[0].session_id.clone();
    e.attach_recording(&id, "r").unwrap();
    e.recording_finished("r");
    update(&mut e, CallState::ConfirmedCall, 4000);
    assert!(e.prompts().is_empty());
}
#[test]
fn permissions_are_unknown_not_call_end() {
    let mut e = Engine::default();
    update(&mut e, CallState::ConfirmedCall, 0);
    update(&mut e, CallState::ConfirmedCall, 3000);
    update(&mut e, CallState::NoCall, 4000);
    update(&mut e, CallState::Unknown, 24000);
    assert_ne!(e.sessions()[0].phase, Phase::Ended);
}
#[test]
fn pid_reuse_invalidates_old_action() {
    let mut e = Engine::default();
    update(&mut e, CallState::ConfirmedCall, 0);
    update(&mut e, CallState::ConfirmedCall, 3000);
    let id = e.sessions()[0].session_id.clone();
    let mut o = observation(CallState::ConfirmedCall, 4000);
    o.process_identity = Some("10-creation2".into());
    e.update(&[o], &settings(), 4000);
    assert!(e.authorize_start(&id).is_err());
}
#[test]
fn audio_or_process_alone_never_confirm() {
    assert!(!call_control_matches(
        "zoom",
        &["Test microphone".into()],
        &[]
    ));
    assert!(!call_control_matches(
        "discord",
        &["Disconnect".into()],
        &[]
    ));
    assert!(call_control_matches(
        "discord",
        &["Disconnect".into()],
        &["Voice Connected".into()]
    ));
    assert!(!call_control_matches(
        "browser",
        &["Leave meeting".into()],
        &[]
    ));
}

#[test]
fn unknown_breaks_contiguous_debounce() {
    let mut e = Engine::default();
    update(&mut e, CallState::ConfirmedCall, 0);
    update(&mut e, CallState::Unknown, 1000);
    update(&mut e, CallState::ConfirmedCall, 5000);
    assert!(e.prompts().is_empty());
    update(&mut e, CallState::ConfirmedCall, 7999);
    assert!(e.prompts().is_empty());
    update(&mut e, CallState::ConfirmedCall, 8000);
    assert_eq!(e.prompts().len(), 1);
}
#[test]
fn returned_call_invalidates_pending_stop() {
    let mut e = Engine::default();
    update(&mut e, CallState::ConfirmedCall, 0);
    update(&mut e, CallState::ConfirmedCall, 3000);
    let id = e.sessions()[0].session_id.clone();
    e.attach_recording(&id, "owned").unwrap();
    update(&mut e, CallState::NoCall, 4000);
    update(&mut e, CallState::NoCall, 24000);
    assert!(e.authorize_stop(&id, Some("owned")).is_ok());
    update(&mut e, CallState::ConfirmedCall, 25000);
    assert!(e.authorize_stop(&id, Some("owned")).is_err());
    assert!(e.prompts().is_empty());
}
#[test]
fn unknown_after_ended_invalidates_stop() {
    let mut e = Engine::default();
    update(&mut e, CallState::ConfirmedCall, 0);
    update(&mut e, CallState::ConfirmedCall, 3000);
    let id = e.sessions()[0].session_id.clone();
    e.attach_recording(&id, "owned").unwrap();
    update(&mut e, CallState::NoCall, 4000);
    update(&mut e, CallState::NoCall, 24000);
    update(&mut e, CallState::Unknown, 25000);
    assert!(e.authorize_stop(&id, Some("owned")).is_err());
    assert!(e.prompts().is_empty());
}
#[test]
fn configurable_intervals_and_simultaneous_calls() {
    let mut e = Engine::default();
    let s = Settings {
        enabled: true,
        debounce_ms: 1000,
        grace_ms: 5000,
        ..Settings::default()
    };
    let z = observation(CallState::ConfirmedCall, 0);
    let mut d = z.clone();
    d.application = "discord".into();
    d.process_identity = Some("11-creation".into());
    e.update(&[z.clone(), d.clone()], &s, 0);
    assert!(e.prompts().is_empty());
    e.update(&[z, d], &s, 1000);
    assert_eq!(e.prompts().len(), 2);
    assert!(e.sessions().iter().all(|s| s.recording_id.is_none()));
}
#[test]
fn deselection_revokes_pending_start() {
    let mut e = Engine::default();
    update(&mut e, CallState::ConfirmedCall, 0);
    update(&mut e, CallState::ConfirmedCall, 3000);
    let id = e.sessions()[0].session_id.clone();
    let s = Settings {
        enabled: true,
        applications: vec!["discord".into()],
        ..Settings::default()
    };
    e.update(&[observation(CallState::NoCall, 5000)], &s, 5000);
    assert!(e.authorize_start(&id).is_err());
    assert!(e.prompts().is_empty());
}
#[test]
fn reserved_start_cannot_follow_reused_process() {
    let mut e = Engine::default();
    update(&mut e, CallState::ConfirmedCall, 0);
    update(&mut e, CallState::ConfirmedCall, 3000);
    let id = e.sessions()[0].session_id.clone();
    e.reserve_start(&id).unwrap();
    assert!(e.validate_reserved_start(&id, Some("10-creation1")).is_ok());
    let mut o = observation(CallState::ConfirmedCall, 4000);
    o.process_identity = Some("10-creation2".into());
    e.update(&[o.clone()], &settings(), 4000);
    assert!(e
        .validate_reserved_start(&id, o.process_identity.as_deref())
        .is_err());
}
#[test]
fn reserved_start_after_confirmed_exit_does_not_join_new_call() {
    let mut e = Engine::default();
    update(&mut e, CallState::ConfirmedCall, 0);
    update(&mut e, CallState::ConfirmedCall, 3000);
    let id = e.sessions()[0].session_id.clone();
    e.reserve_start(&id).unwrap();
    update(&mut e, CallState::NoCall, 4000);
    update(&mut e, CallState::NoCall, 24000);
    update(&mut e, CallState::ConfirmedCall, 25000);
    assert!(e
        .validate_reserved_start(&id, Some("10-creation1"))
        .is_err());
}
#[test]
fn dismissed_stop_offer_does_not_repeat() {
    let mut e = Engine::default();
    update(&mut e, CallState::ConfirmedCall, 0);
    update(&mut e, CallState::ConfirmedCall, 3000);
    let id = e.sessions()[0].session_id.clone();
    e.attach_recording(&id, "owned").unwrap();
    update(&mut e, CallState::NoCall, 4000);
    update(&mut e, CallState::NoCall, 24000);
    assert_eq!(e.prompts().len(), 1);
    e.suppress(&id).unwrap();
    update(&mut e, CallState::NoCall, 25000);
    assert!(e.prompts().is_empty());
}

#[test]
fn transient_focus_loss_keeps_offer_without_authorizing_capture() {
    let mut e = Engine::default();
    update(&mut e, CallState::ConfirmedCall, 0);
    update(&mut e, CallState::ConfirmedCall, 3000);
    let id = e.sessions()[0].session_id.clone();
    update(&mut e, CallState::Unknown, 4000);
    assert_eq!(e.sessions()[0].phase, Phase::Uncertain);
    assert_eq!(
        e.prompts()[0].session_id,
        id,
        "temporary AX loss must not erase the user's action"
    );
    assert!(
        e.authorize_start(&id).is_err(),
        "retained offer is not capture authorization"
    );
    update(&mut e, CallState::ConfirmedCall, 5000);
    assert!(e.authorize_start(&id).is_ok());
}

#[test]
fn prolonged_unknown_does_not_keep_a_start_offer_forever() {
    let mut e = Engine::default();
    update(&mut e, CallState::ConfirmedCall, 0);
    update(&mut e, CallState::ConfirmedCall, 3000);
    update(&mut e, CallState::Unknown, 4000);
    assert_eq!(e.prompts().len(), 1);
    update(&mut e, CallState::Unknown, 24000);
    assert!(e.prompts().is_empty());
}

#[test]
fn retained_focus_offer_still_requires_same_process_and_rejects_positive_exit() {
    let mut e = Engine::default();
    update(&mut e, CallState::ConfirmedCall, 0);
    update(&mut e, CallState::ConfirmedCall, 3000);
    let id = e.sessions()[0].session_id.clone();
    update(&mut e, CallState::Unknown, 4000);
    assert_eq!(e.revalidation_identity(&id).unwrap(), "10-creation1");
    assert!(e.validate_identity(&id, Some("10-creation2")).is_err());
    update(&mut e, CallState::NoCall, 4500);
    assert!(e.revalidation_identity(&id).is_err());
    assert!(e.authorize_start(&id).is_err());
}
