//! Thin keyring facade over the `keyring` crate (Secret Service on
//! Linux, pure-Rust D-Bus — no C dependency). Every rtv secret lives
//! under service "io.github.tunix.rtv" with the per-source name as the
//! user attribute. AGENTS.md hardline: credentials NEVER touch disk in
//! plain text — the keyring is the only place besides process memory.

use keyring::Entry;

const SERVICE: &str = "io.github.tunix.rtv";

fn entry(name: &str) -> Result<Entry, keyring::Error> {
    Entry::new(SERVICE, name)
}

pub fn store(name: &str, secret: &str) -> Result<(), keyring::Error> {
    let entry = entry(name)?;
    // set_password overwrites an existing secret; a stale one from a
    // previous run with the same name would otherwise linger.
    entry.set_password(secret)
}

pub fn lookup(name: &str) -> Result<Option<String>, keyring::Error> {
    match entry(name)?.get_password() {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(err) => Err(err),
    }
}

pub fn delete(name: &str) -> Result<(), keyring::Error> {
    match entry(name)?.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(err),
    }
}
