//! Secrets in the OS keychain (Secret Service on Linux, Credential Manager on
//! Windows, Keychain on macOS) instead of settings.json.

use crate::error::{AppError, AppResult};

const SERVICE: &str = "app.tunewire.player";

fn entry(name: &str) -> AppResult<keyring::Entry> {
    keyring::Entry::new(SERVICE, name).map_err(|e| AppError::Storage(format!("keychain unavailable: {e}")))
}

pub fn get(name: &str) -> Option<String> {
    #[cfg(test)]
    return fake::get(name);
    #[allow(unreachable_code)]
    entry(name).ok()?.get_password().ok().filter(|s| !s.is_empty())
}

pub fn set(name: &str, value: &str) -> AppResult<()> {
    #[cfg(test)]
    return fake::set(name, value);
    #[allow(unreachable_code)]
    let e = entry(name)?;
    if value.is_empty() {
        match e.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(AppError::Storage(format!("keychain: {err}"))),
        }
    } else {
        e.set_password(value).map_err(|err| AppError::Storage(format!("could not save to the system keychain: {err}")))
    }
}

pub const SPOTIFY_SECRET: &str = "spotify_client_secret";

/// Tests never touch the real keychain: an in-memory one that can be made
/// to fail like a locked or missing keychain.
#[cfg(test)]
pub mod fake {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    static STORE: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);
    pub static BROKEN: AtomicBool = AtomicBool::new(false);

    pub fn get(name: &str) -> Option<String> {
        if BROKEN.load(Ordering::SeqCst) {
            return None;
        }
        STORE.lock().unwrap().as_ref()?.get(name).cloned().filter(|s| !s.is_empty())
    }

    pub fn set(name: &str, value: &str) -> crate::error::AppResult<()> {
        if BROKEN.load(Ordering::SeqCst) {
            return Err(crate::error::AppError::Storage("keychain locked".into()));
        }
        STORE.lock().unwrap().get_or_insert_with(HashMap::new).insert(name.into(), value.into());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore]
    fn keychain_roundtrip() {
        // The real keychain (get/set use an in-memory one in tests).
        let e = super::entry("selftest").expect("keychain");
        e.set_password("s3cr3t").expect("set");
        assert_eq!(e.get_password().ok().as_deref(), Some("s3cr3t"));
        e.delete_credential().expect("delete");
        assert!(e.get_password().is_err());
    }
}
