//! Cloud API keys in the macOS Keychain.
//!
//! Keys are written from the Voice engine or Polish screens and read when
//! an engine or polish request is built. They never go back to the web view:
//! the UI learns only whether a key is stored.

use std::sync::Arc;

use speechkit::Secret;

const SERVICE: &str = "app.viary.api-key";

/// A cloud provider whose key Viary stores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    OpenAi,
    DashScope,
    /// Independent credentials for a custom polish server.
    CustomPolish,
}

impl Provider {
    fn account(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::DashScope => "dashscope",
            Self::CustomPolish => "custom-polish",
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use security_framework::passwords::{
        delete_generic_password, get_generic_password, set_generic_password,
    };

    use super::SERVICE;

    pub(super) fn set(account: &str, value: &str) -> Result<(), String> {
        set_generic_password(SERVICE, account, value.as_bytes()).map_err(|e| e.to_string())
    }

    pub(super) fn get(account: &str) -> Option<String> {
        let bytes = get_generic_password(SERVICE, account).ok()?;
        String::from_utf8(bytes).ok()
    }

    pub(super) fn delete(account: &str) -> Result<(), String> {
        match delete_generic_password(SERVICE, account) {
            Ok(()) => Ok(()),
            // errSecItemNotFound: nothing to delete.
            Err(error) if error.code() == -25300 => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    pub(super) fn set(_: &str, _: &str) -> Result<(), String> {
        Err("the Keychain is only available on macOS".into())
    }
    pub(super) fn get(_: &str) -> Option<String> {
        None
    }
    pub(super) fn delete(_: &str) -> Result<(), String> {
        Ok(())
    }
}

/// Stores `key` for `provider`, replacing any earlier one.
pub fn save(provider: Provider, key: &str) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("the API key is empty".into());
    }
    imp::set(provider.account(), key)
}

/// The stored key, wrapped so it is never printed.
pub fn load(provider: Provider) -> Option<Arc<Secret>> {
    imp::get(provider.account())
        .filter(|key| !key.trim().is_empty())
        .map(|key| Arc::new(Secret::new(key)))
}

pub fn has(provider: Provider) -> bool {
    imp::get(provider.account()).is_some_and(|key| !key.trim().is_empty())
}

pub fn delete(provider: Provider) -> Result<(), String> {
    imp::delete(provider.account())
}
