use call_detection::{merge_latest_observations, CallState, Observation};
#[test]
fn delayed_poll_cannot_overwrite_explicit_start_revalidation() {
    let mut fresh = Observation::unknown("zoom", 5000, "fresh UI check");
    fresh.state = CallState::ConfirmedCall;
    let mut current = vec![fresh];
    let stale = Observation::unknown("zoom", 4000, "toolbar hidden before Start click");
    let accepted = merge_latest_observations(&mut current, vec![stale]);
    assert!(accepted.is_empty());
    assert_eq!(current[0].state, CallState::ConfirmedCall);
    let mut exited = Observation::unknown("zoom", 6000, "");
    exited.state = CallState::NoCall;
    assert_eq!(
        merge_latest_observations(&mut current, vec![exited]).len(),
        1
    );
    assert_eq!(current[0].state, CallState::NoCall);
}
