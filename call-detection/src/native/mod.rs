#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;
#[cfg(not(any(target_os = "macos", windows)))]
use crate::CallState;
use crate::{Observation, Observer};

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
/// Explicit-action revalidation never accepts cached process or audio evidence.
pub fn revalidate(application: &str, identity: &str, at: u64) -> Result<Observation, String> {
    #[cfg(target_os = "macos")]
    {
        macos::revalidate(application, identity, at)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let observation = create()
            .observe(at)
            .into_iter()
            .find(|o| o.application == application)
            .ok_or("Application observation unavailable")?;
        if observation.process_identity.as_deref() != Some(identity) {
            return Err("Call application exited or its process identity changed".into());
        }
        Ok(observation)
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
