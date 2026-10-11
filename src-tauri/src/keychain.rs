//! Cloud API keys in the macOS Keychain.
//!
//! Keys are written from the Voice engine or Polish screens and read when
//! an engine or polish request is built. They never go back to the web view:
//! the UI learns only whether a key is stored.

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use speechkit::Secret;

use crate::lock;

const SERVICE: &str = "app.viary.api-key";

/// What the last read or write of each provider's key found, by
/// [`Provider`] order. Every snapshot and tray menu asks, and Credential
/// Manager and the Secret Service answer over IPC (the Secret Service may
/// even ask to unlock the keyring). Keys change through Viary, which
/// updates this.
static HAS: Mutex<[Option<Known>; 3]> = Mutex::new([None; 3]);

#[derive(Clone, Copy)]
enum Known {
    /// Whether there is a key.
    Has(bool),
    /// When the store last did not answer: a locked keyring, a dismissed
    /// unlock prompt, the Secret Service not up yet at login. Not "no
    /// key": asked again after [`ASK_AGAIN_AFTER`].
    Unanswered(Instant),
}

/// How long a store that did not answer is left alone before asking again,
/// so an unlock prompt does not come back with every snapshot.
const ASK_AGAIN_AFTER: Duration = Duration::from_secs(60);

fn remember(provider: Provider, has: bool) {
    lock(&HAS)[provider as usize] = Some(Known::Has(has));
}

/// Reads `provider`'s key from the store and remembers what it found.
fn read(provider: Provider) -> Result<Option<String>, String> {
    match imp::get(provider.account()) {
        Ok(key) => {
            let key = key.filter(|key| !key.trim().is_empty());
            remember(provider, key.is_some());
            Ok(key)
        }
        Err(error) => {
            tracing::warn!(%error, account = provider.account(), "cannot read the API key");
            lock(&HAS)[provider as usize] = Some(Known::Unanswered(Instant::now()));
            Err(error)
        }
    }
}

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

    /// The stored key; `Ok(None)` when there is none (or it is not text).
    pub(super) fn get(account: &str) -> Result<Option<String>, String> {
        match get_generic_password(SERVICE, account) {
            Ok(bytes) => Ok(String::from_utf8(bytes).ok()),
            // errSecItemNotFound: no key.
            Err(error) if error.code() == -25300 => Ok(None),
            Err(error) => Err(error.to_string()),
        }
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

/// Windows Credential Manager and the Linux Secret Service (GNOME
/// Keyring), under the same service and account names.
#[cfg(not(target_os = "macos"))]
mod imp {
    use keyring::{Entry, Error};

    use super::SERVICE;

    fn entry(account: &str) -> Result<Entry, String> {
        Entry::new(SERVICE, account).map_err(|e| e.to_string())
    }

    pub(super) fn set(account: &str, value: &str) -> Result<(), String> {
        entry(account)?.set_password(value).map_err(|e| e.to_string())
    }

    /// The stored key; `Ok(None)` when there is none (or it is not text).
    pub(super) fn get(account: &str) -> Result<Option<String>, String> {
        match entry(account)?.get_password() {
            Ok(key) => Ok(Some(key)),
            Err(Error::NoEntry | Error::BadEncoding(_)) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    pub(super) fn delete(account: &str) -> Result<(), String> {
        match entry(account)?.delete_credential() {
            Ok(()) | Err(Error::NoEntry) => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

/// Stores `key` for `provider`, replacing any earlier one.
pub fn save(provider: Provider, key: &str) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("the API key is empty".into());
    }
    imp::set(provider.account(), key)?;
    remember(provider, true);
    Ok(())
}

/// The stored key, wrapped so it is never printed.
pub fn load(provider: Provider) -> Option<Arc<Secret>> {
    read(provider).ok().flatten().map(|key| Arc::new(Secret::new(key)))
}

/// Whether a key is stored. While the store does not answer, false, and
/// asked again a minute later.
pub fn has(provider: Provider) -> bool {
    let known = lock(&HAS)[provider as usize];
    match known {
        Some(Known::Has(has)) => has,
        Some(Known::Unanswered(at)) if at.elapsed() < ASK_AGAIN_AFTER => false,
        _ => read(provider).is_ok_and(|key| key.is_some()),
    }
}

pub fn delete(provider: Provider) -> Result<(), String> {
    imp::delete(provider.account())?;
    remember(provider, false);
    Ok(())
}
