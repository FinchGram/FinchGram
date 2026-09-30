//! Release signing tool: the Ed25519 key pair behind FinchGram's updates.
//!
//! The public half is built into the app (`release-signing.pub`, read by src/update.rs); the
//! private half exists only as the `RELEASE_SIGNING_KEY` secret of the GitHub Actions workflow,
//! with a backup in the release manager's macOS keychain. CI signs `SHA256SUMS` of every release
//! and publishes the signature as `SHA256SUMS.sig`; the app refuses updates whose signature does
//! not verify. Keys and signatures are hex; a private key is the 32-byte Ed25519 seed.
//!
//!   finchgram-release-sign keygen <private-key-file>      # writes the private key there (mode 600), prints the public key
//!   finchgram-release-sign sign <file>                     # RELEASE_SIGNING_KEY=<hex> in the environment; writes <file>.sig
//!   finchgram-release-sign verify <file> <public-keys-file>   # checks <file>.sig against the keys in that file (what the app does)

use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::process::ExitCode;

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["keygen", path] => keygen(path),
        ["sign", path] => sign(path),
        ["verify", path, keys] => verify(path, keys),
        _ => Err("usage: finchgram-release-sign keygen <private-key-file> | sign <file> | verify <file> <public-keys-file>".to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("finchgram-release-sign: {err}");
            ExitCode::FAILURE
        }
    }
}

fn keygen(path: &str) -> Result<(), String> {
    let mut seed = [0u8; 32];
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut seed))
        .map_err(|err| format!("cannot read /dev/urandom: {err}"))?;
    let key = SigningKey::from_bytes(&seed);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|err| format!("cannot create {path}: {err}"))?;
    use std::io::Write;
    writeln!(file, "{}", hex::encode(seed)).map_err(|err| format!("cannot write {path}: {err}"))?;
    println!("{}", hex::encode(key.verifying_key().to_bytes()));
    Ok(())
}

fn sign(path: &str) -> Result<(), String> {
    let key_hex = std::env::var("RELEASE_SIGNING_KEY")
        .map_err(|_| "RELEASE_SIGNING_KEY is not set in the environment".to_string())?;
    let seed: [u8; 32] = hex::decode(key_hex.trim())
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| "RELEASE_SIGNING_KEY is not a 32-byte hex key".to_string())?;
    let key = SigningKey::from_bytes(&seed);
    let message = fs::read(path).map_err(|err| format!("cannot read {path}: {err}"))?;
    let signature = key.sign(&message);
    let sig_path = format!("{path}.sig");
    fs::write(&sig_path, format!("{}\n", hex::encode(signature.to_bytes())))
        .map_err(|err| format!("cannot write {sig_path}: {err}"))?;
    println!("signed {path} -> {sig_path} (key {})", hex::encode(key.verifying_key().to_bytes()));
    Ok(())
}

fn verify(path: &str, keys_path: &str) -> Result<(), String> {
    let message = fs::read(path).map_err(|err| format!("cannot read {path}: {err}"))?;
    let sig_path = format!("{path}.sig");
    let signature = fs::read_to_string(&sig_path).map_err(|err| format!("cannot read {sig_path}: {err}"))?;
    let keys = fs::read_to_string(keys_path).map_err(|err| format!("cannot read {keys_path}: {err}"))?;
    let keys: Vec<&str> = public_keys(&keys);
    verify_with(&message, &signature, &keys)?;
    println!("{sig_path} verifies against a key in {keys_path}");
    Ok(())
}

/// The keys listed in a public-keys file: one hex key per line, `#` comments and blanks ignored.
/// (Kept identical to the copy in src/update.rs.)
fn public_keys(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect()
}

/// (Kept identical to the copy in src/update.rs.)
fn verify_with(message: &[u8], signature_hex: &str, keys_hex: &[&str]) -> Result<(), String> {
    let bytes = hex::decode(signature_hex.trim()).map_err(|err| format!("signature is not hex: {err}"))?;
    let signature = Signature::from_slice(&bytes).map_err(|err| format!("not an Ed25519 signature: {err}"))?;
    if keys_hex.is_empty() {
        return Err("no release signing key is configured".to_string());
    }
    for key_hex in keys_hex {
        let key_bytes: [u8; 32] = hex::decode(key_hex)
            .ok()
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or_else(|| format!("public key {key_hex:?} is not a 32-byte hex key"))?;
        let key = VerifyingKey::from_bytes(&key_bytes).map_err(|err| format!("public key {key_hex:?} is invalid: {err}"))?;
        if key.verify_strict(message, &signature).is_ok() {
            return Ok(());
        }
    }
    Err("the signature does not match any release signing key".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_then_verify_and_reject_tampering() {
        let key = SigningKey::from_bytes(&[7u8; 32]);
        let public = hex::encode(key.verifying_key().to_bytes());
        let message = b"abc  FinchGram-0.1.3-macos-arm64.zip\n";
        let signature = hex::encode(key.sign(message).to_bytes());

        assert!(verify_with(message, &signature, &[public.as_str()]).is_ok());
        assert!(verify_with(b"tampered", &signature, &[public.as_str()]).is_err());
        let other = hex::encode(SigningKey::from_bytes(&[9u8; 32]).verifying_key().to_bytes());
        assert!(verify_with(message, &signature, &[other.as_str()]).is_err());
        assert!(verify_with(message, &signature, &[]).is_err());
        assert!(verify_with(message, "zz", &[public.as_str()]).is_err());
    }

    #[test]
    fn keys_file_ignores_comments_and_blanks() {
        assert_eq!(public_keys("# a comment\n\n  aabb  \n#x\nccdd\n"), vec!["aabb", "ccdd"]);
    }
}
