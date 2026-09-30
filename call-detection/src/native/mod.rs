#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;
use crate::Observer;
#[cfg(not(any(target_os = "macos", windows)))]
use crate::{CallState, Observation};

pub fn create() -> Box<dyn Observer> {
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::NativeObserver::new())
    }
    #[cfg(windows)]
    {
        Box::new(windows::NativeObserver::new())
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Box::new(Unsupported)
    }
}
#[cfg(not(any(target_os = "macos", windows)))]
struct Unsupported;
#[cfg(not(any(target_os = "macos", windows)))]
impl Observer for Unsupported {
    fn observe(&mut self, at: u64) -> Vec<Observation> {
        ["zoom", "discord", "teams", "browser"]
            .iter()
            .map(|a| {
                let mut o = Observation::unknown(
                    a,
                    at,
                    "Native call detection is available only on macOS and Windows",
                );
                o.state = CallState::Unsupported;
                o
            })
            .collect()
    }
}
