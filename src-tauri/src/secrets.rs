//! Secrets in the OS keychain (Secret Service on Linux, Credential Manager on
//! Windows, Keychain on macOS) instead of settings.json.

use crate::error::{AppError, AppResult};

const SERVICE: &str = "app.tunewire.player";

fn entry(name: &str) -> AppResult<keyring::Entry> {
    keyring::Entry::new(SERVICE, name).map_err(|e| AppError::Storage(format!("keychain unavailable: {e}")))
}

pub fn get(name: &str) -> Option<String> {
    entry(name).ok()?.get_password().ok().filter(|s| !s.is_empty())
}

pub fn set(name: &str, value: &str) -> AppResult<()> {
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

#[cfg(test)]
mod tests {
    #[test]
    #[ignore]
    fn keychain_roundtrip() {
        super::set("selftest", "s3cr3t").expect("set");
        assert_eq!(super::get("selftest").as_deref(), Some("s3cr3t"));
        super::set("selftest", "").expect("delete");
        assert_eq!(super::get("selftest"), None);
    }
}
