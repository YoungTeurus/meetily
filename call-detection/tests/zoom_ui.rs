use call_detection::{classify_zoom_controls, CallState};
#[test]
fn focus_hidden_toolbar_is_unknown_without_enabled_call_command() {
    assert_eq!(classify_zoom_controls(&[], None), CallState::Unknown);
}
#[test]
fn enabled_meeting_menu_confirms_when_toolbar_is_hidden() {
    assert_eq!(
        classify_zoom_controls(&[], Some(&[("Leave Meeting…".into(), true)])),
        CallState::ConfirmedCall
    );
}
#[test]
fn disabled_leave_meeting_does_not_confirm() {
    assert_eq!(
        classify_zoom_controls(&[], Some(&[("Leave Meeting…".into(), false)])),
        CallState::Unknown
    );
}
#[test]
fn inactive_zoom_home_is_positive_exit_only_with_readable_menu() {
    let home = ["New Meeting".into(), "Join".into()];
    assert_eq!(classify_zoom_controls(&home, None), CallState::Unknown);
    assert_eq!(classify_zoom_controls(&home, Some(&[])), CallState::Unknown);
}
#[test]
fn explicit_disabled_leave_and_readable_idle_controls_confirm_exit() {
    let home = ["New Meeting".into(), "Join".into()];
    assert_eq!(
        classify_zoom_controls(&home, Some(&[("Leave Meeting".into(), false)])),
        CallState::NoCall
    );
}
#[test]
fn home_alongside_hidden_call_window_is_not_positive_exit() {
    let home = ["New Meeting".into(), "Join".into()];
    assert_eq!(
        call_detection::classify_zoom_window_set(
            &home,
            Some(&[("Leave Meeting".into(), false)]),
            2
        ),
        CallState::Unknown
    );
    assert_eq!(
        call_detection::classify_zoom_window_set(&home, None, 1),
        CallState::Unknown
    );
}
#[test]
fn disabled_host_end_does_not_imply_participant_left() {
    assert_eq!(
        classify_zoom_controls(&[], Some(&[("End Meeting".into(), false)])),
        CallState::Unknown
    );
}
#[test]
fn microphone_test_and_audio_alone_cannot_confirm() {
    assert_eq!(
        classify_zoom_controls(&["Test Microphone".into()], Some(&[])),
        CallState::Unknown
    );
}
#[test]
fn russian_zoom_menu_commands_survive_focus_hidden_toolbar() {
    for label in [
        "Выйти из конференции…",
        "Покинуть конференцию...",
        "Завершить конференцию",
        "Завершить конференцию для всех",
    ] {
        assert_eq!(
            classify_zoom_controls(&[], Some(&[(label.into(), true)])),
            CallState::ConfirmedCall,
            "{label}"
        );
    }
}
