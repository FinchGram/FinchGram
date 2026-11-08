//! The roots the Keychain trusts, as a PEM file for mpv's TLS (mbedTLS has no store of its own):
//! the system's, and those an administrator or the user trusted, as every other connection of the
//! app trusts them (rustls's platform verifier). Written once a run into the caches folder.

use std::cell::RefCell;
use std::path::PathBuf;

use base64::Engine as _;

thread_local! {
    static WRITTEN: RefCell<Option<Option<PathBuf>>> = const { RefCell::new(None) };
}

/// The PEM file, written at the first call of the run; None when the Keychain gave nothing or the
/// file could not be written (the stream then fails its certificate check rather than play
/// unchecked).
pub fn file() -> Option<PathBuf> {
    WRITTEN.with(|written| {
        if let Some(path) = written.borrow().as_ref() {
            return path.clone();
        }
        let path = write().map_err(|err| eprintln!("live: the Keychain's roots cannot be given to mpv: {err}")).ok();
        *written.borrow_mut() = Some(path.clone());
        path
    })
}

fn write() -> Result<PathBuf, String> {
    let found = rustls_native_certs::load_native_certs();
    if found.certs.is_empty() {
        return Err(format!("the Keychain gave no roots ({} errors)", found.errors.len()));
    }
    let mut pem = String::new();
    for cert in &found.certs {
        let encoded = base64::engine::general_purpose::STANDARD.encode(cert.as_ref());
        pem.push_str("-----BEGIN CERTIFICATE-----\n");
        for line in encoded.as_bytes().chunks(64) {
            pem.push_str(std::str::from_utf8(line).unwrap_or_default());
            pem.push('\n');
        }
        pem.push_str("-----END CERTIFICATE-----\n");
    }
    let folder = dirs::cache_dir().ok_or("no caches folder")?.join("FinchGram");
    std::fs::create_dir_all(&folder).map_err(|err| err.to_string())?;
    let path = folder.join("keychain-roots.pem");
    std::fs::write(&path, pem).map_err(|err| err.to_string())?;
    Ok(path)
}
