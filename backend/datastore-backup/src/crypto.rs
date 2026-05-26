//! age-encryption wrapper. Reads recipient(s) from a `.pub` file and the
//! identity from a `.key` file. We deliberately use the `age` crate directly
//! (not shelling out to /usr/bin/age) so the binary works in restricted
//! systemd sandboxes without PATH leakage.

use age::{Decryptor, Encryptor, Identity, Recipient};
use anyhow::{anyhow, Context, Result};
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::str::FromStr;

/// Load one or more age recipients from a `.pub`-style file. Lines that are
/// blank or start with `#` are ignored, so a key file copy-pasted from
/// `age-keygen` output also works.
pub fn load_recipient(path: &Path) -> Result<Vec<Box<dyn Recipient + Send>>> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("read recipient {}", path.display()))?;
    let mut recipients: Vec<Box<dyn Recipient + Send>> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Some users keep "Public key: age1..." style headers.
        let key = line.rsplit(':').next().unwrap_or(line).trim();
        if !key.starts_with("age1") {
            continue;
        }
        let r = age::x25519::Recipient::from_str(key)
            .map_err(|e| anyhow!("invalid age recipient: {e}"))?;
        recipients.push(Box::new(r));
    }
    if recipients.is_empty() {
        return Err(anyhow!("no age recipients found in {}", path.display()));
    }
    Ok(recipients)
}

/// Load the age identity from a `.key` file (the secret key file). Strict
/// mode: must contain at least one `AGE-SECRET-KEY-...` line.
pub fn load_identity(path: &Path) -> Result<age::x25519::Identity> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("read identity {}", path.display()))?;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("AGE-SECRET-KEY-") {
            return age::x25519::Identity::from_str(line)
                .map_err(|e| anyhow!("invalid age identity: {e}"));
        }
    }
    Err(anyhow!("no AGE-SECRET-KEY in {}", path.display()))
}

/// Encrypt `plaintext` to `dst` using all `recipients`. Streaming so a 50 MB
/// nightly export never lives fully in RAM twice.
pub fn encrypt_to_writer<R: Read, W: Write>(
    mut plaintext: R,
    dst: W,
    recipients: Vec<Box<dyn Recipient + Send>>,
) -> Result<()> {
    let encryptor = Encryptor::with_recipients(recipients)
        .ok_or_else(|| anyhow!("no recipients supplied"))?;
    let mut writer = encryptor.wrap_output(dst)?;
    std::io::copy(&mut plaintext, &mut writer)?;
    writer.finish()?;
    Ok(())
}

/// Decrypt an age stream using `identity`.
pub fn decrypt_to_writer<R: Read, W: Write>(
    ciphertext: R,
    mut dst: W,
    identity: &dyn Identity,
) -> Result<()> {
    let decryptor = match Decryptor::new(ciphertext)? {
        Decryptor::Recipients(d) => d,
        Decryptor::Passphrase(_) => return Err(anyhow!("passphrase-encrypted, not supported")),
    };
    let mut reader = decryptor.decrypt(std::iter::once(identity as &dyn Identity))?;
    std::io::copy(&mut reader, &mut dst)?;
    Ok(())
}
