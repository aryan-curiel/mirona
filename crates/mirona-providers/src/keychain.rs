//! API keys in the OS keychain, never in config files (NGR-145).
//!
//! One entry per provider under the service "mirona" (macOS Keychain, Windows Credential
//! Manager, Linux Secret Service). `config.toml` only names the provider.

use crate::{Error, Result};

pub const SERVICE: &str = "mirona";

fn entry(provider: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, provider).map_err(|e| Error::Provider(format!("keychain: {e}")))
}

pub fn set_api_key(provider: &str, key: &str) -> Result<()> {
    entry(provider)?
        .set_password(key)
        .map_err(|e| Error::Provider(format!("keychain: {e}")))
}

/// `None` when no key was saved for this provider.
pub fn api_key(provider: &str) -> Result<Option<String>> {
    match entry(provider)?.get_password() {
        Ok(k) => Ok(Some(k)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(Error::Provider(format!("keychain: {e}"))),
    }
}

/// Removing a key that isn't there is fine.
pub fn delete_api_key(provider: &str) -> Result<()> {
    match entry(provider)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(Error::Provider(format!("keychain: {e}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Touches the real OS keychain. `cargo test -p mirona-providers --features keychain -- --ignored`
    #[test]
    #[ignore = "writes to the real OS keychain"]
    fn round_trips_a_key() {
        let p = "mirona-test-provider";
        set_api_key(p, "sk-test-123").unwrap();
        assert_eq!(api_key(p).unwrap().as_deref(), Some("sk-test-123"));
        delete_api_key(p).unwrap();
        assert_eq!(api_key(p).unwrap(), None);
        delete_api_key(p).unwrap();
    }
}
