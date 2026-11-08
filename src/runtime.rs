//! Platform selection for the computer-use runtime.

/// What one observation returns: the text an agent reads, plus an optional
/// screenshot kept separate so it never inflates the text.
pub struct State {
    pub text: String,
    pub image: Option<Vec<u8>>,
}

#[cfg(target_os = "macos")]
pub use crate::macos::Runtime;

#[cfg(not(target_os = "macos"))]
mod unsupported {
    use anyhow::{bail, Result};
    use serde_json::Value;

    use super::State;

    pub struct Runtime;

    impl Default for Runtime {
        fn default() -> Self {
            Runtime
        }
    }

    const MESSAGE: &str =
        "computer use is only implemented on macOS; this build was compiled for another platform";

    impl Runtime {
        pub fn new() -> Self {
            Runtime
        }

        fn unsupported<T>() -> Result<T> {
            bail!(MESSAGE)
        }

        pub fn guard(&self) -> Result<()> {
            Self::unsupported()
        }

        pub fn list_apps(&self, _include_helpers: bool) -> Result<String> {
            Self::unsupported()
        }

        pub fn get_app_state(&mut self, _: &Value) -> Result<State> {
            Self::unsupported()
        }

        pub fn query(&mut self, _: &Value) -> Result<String> {
            Self::unsupported()
        }

        pub fn click(&mut self, _: &Value) -> Result<String> {
            Self::unsupported()
        }

        pub fn perform_action(&mut self, _: &Value) -> Result<String> {
            Self::unsupported()
        }

        pub fn set_value(&mut self, _: &Value) -> Result<String> {
            Self::unsupported()
        }

        pub fn select_text(&mut self, _: &Value) -> Result<String> {
            Self::unsupported()
        }

        pub fn type_text(&mut self, _: &Value) -> Result<String> {
            Self::unsupported()
        }

        pub fn press_key(&mut self, _: &Value) -> Result<String> {
            Self::unsupported()
        }

        pub fn scroll(&mut self, _: &Value) -> Result<String> {
            Self::unsupported()
        }

        pub fn drag(&mut self, _: &Value) -> Result<String> {
            Self::unsupported()
        }

        pub fn screenshot(&self, _: &Value) -> Result<Vec<u8>> {
            Self::unsupported()
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub use unsupported::Runtime;
