use anyhow::{Context, Result, ensure};

/// Secrets stay in Windows Credential Manager / macOS Keychain / Secret Service.
pub struct Credentials;

impl Credentials {
    fn entry(provider: &str) -> Result<keyring::Entry> {
        ensure!(
            !provider.is_empty() && provider.len() <= 200,
            "invalid provider ID"
        );
        keyring::Entry::new("me.cryzo.desktop", provider).context("open credential store")
    }

    pub fn set(provider: &str, secret: &str) -> Result<()> {
        ensure!(!secret.trim().is_empty(), "credential cannot be empty");
        ensure!(
            secret.len() <= 16_384 && !secret.contains(['\r', '\n']),
            "invalid credential format"
        );
        Self::entry(provider)?
            .set_password(secret)
            .context("save credential")
    }

    pub fn get(provider: &str) -> Result<String> {
        Self::entry(provider)?
            .get_password()
            .context("read credential")
    }

    pub fn remove(provider: &str) -> Result<()> {
        match Self::entry(provider)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}
