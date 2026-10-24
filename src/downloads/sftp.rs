//! SFTP for the downloads page, over russh and russh-sftp (SSH in Rust, on the downloads'
//! tokio runtime; remote.rs says who calls what). The account signs in with its password, once
//! the server's key is the one trusted (`KnownKey`: the `ssh_hosts` table of the database, by
//! host and port, as ssh's known_hosts). A server not trusted yet ends the session before the
//! password goes out, with the key it showed (`Error::KeyUnknown`), for the page to ask the user
//! (src/downloads/mod.rs, the design's eighth round); one showing another key than the trusted
//! is refused the same way (`Error::KeyChanged`), as ssh itself does. The sessions of one server
//! opened side by side (a file's chunks, a tree's listings) share the one slot, so that a key
//! trusted reaches them all. A file comes in 8 MB chunks over several sessions at once, each its own SSH connection, the
//! `.part` and `.part.json` of the WebDAV engine marking what is done, so a stopped file goes on
//! where it was. Paths are relative to the folder the account lands in: "host/films" lists
//! "films" there. FinchGram only reads: opendir, stat and read, nothing else.

use std::os::unix::fs::FileExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use russh::client::{self, AuthResult};
use russh::keys::PublicKeyOrCertificate;
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::StatusCode;
use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _};

use super::http::Http;
use super::remote::{Entry, Error};
use super::{log, transfer};

/// How long a connection may take to be ready, and how long a read may bring nothing.
const CONNECT_PATIENCE: Duration = Duration::from_secs(30);
const READ_PATIENCE: u64 = 60;
/// A chunk of a file, as the WebDAV engine cuts it; a piece of a chunk asked for at once.
const CHUNK: u64 = 8 * 1024 * 1024;
const PIECE: usize = 256 * 1024;

/// The key the server is held to, in OpenSSH's one-line form: the one the user trusted, or none
/// until they have. One slot per server (host and port), shared by every `Server` made for it
/// (src/downloads/mod.rs keeps them), so that trusting or forgetting a key reaches the sessions
/// under way.
pub type KnownKey = Arc<Mutex<Option<String>>>;

/// What a key is, for the user to compare: its kind and its SHA256 fingerprint, as ssh-keygen -l
/// prints them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyFacts {
    /// "ED25519", "RSA", "ECDSA" …
    pub kind: String,
    /// "SHA256:…", base64 without padding.
    pub fingerprint: String,
}

/// The facts of a key in OpenSSH's one-line form; none for a line that is not one.
pub fn key_facts(openssh: &str) -> Option<KeyFacts> {
    use russh::keys::{Algorithm, Certificate, HashAlg, PublicKey};
    let (algorithm, fingerprint) = match PublicKey::from_openssh(openssh) {
        Ok(key) => (key.algorithm(), key.fingerprint(HashAlg::Sha256)),
        Err(_) => {
            let certificate = Certificate::from_openssh(openssh).ok()?;
            (certificate.public_key().algorithm(), certificate.public_key().fingerprint(HashAlg::Sha256))
        }
    };
    let kind = match algorithm {
        Algorithm::Ed25519 => "ED25519".to_string(),
        Algorithm::Rsa { .. } => "RSA".to_string(),
        Algorithm::Ecdsa { .. } => "ECDSA".to_string(),
        Algorithm::Dsa => "DSA".to_string(),
        Algorithm::SkEd25519 => "ED25519-SK".to_string(),
        Algorithm::SkEcdsaSha2NistP256 => "ECDSA-SK".to_string(),
        other => other.as_str().to_uppercase(),
    };
    Some(KeyFacts { kind, fingerprint: fingerprint.to_string() })
}

#[derive(Clone)]
pub struct Server {
    pub host: String,
    /// 0 for 22.
    pub port: u16,
    /// The folder under the one the account lands in; none for that one.
    pub folder: Vec<String>,
    pub username: String,
    pub password: String,
    /// The key the server is trusted by; none until the user has trusted one, and then no
    /// session gets as far as the password.
    pub known_key: KnownKey,
}

impl Server {
    pub fn port(&self) -> u16 {
        if self.port == 0 { 22 } else { self.port }
    }

    fn remote_path(&self, path: &[String], name: Option<&str>) -> String {
        let mut parts: Vec<&str> = self.folder.iter().map(String::as_str).collect();
        parts.extend(path.iter().map(String::as_str));
        if let Some(name) = name {
            parts.push(name);
        }
        if parts.is_empty() { ".".to_string() } else { parts.join("/") }
    }

    /// The address as a link to copy: "sftp://me@host:2222/films/2026/" or ".../film.mkv".
    pub fn link(&self, path: &[String], name: Option<&str>) -> String {
        let account = if self.username.is_empty() { String::new() } else { format!("{}@", self.username) };
        let port = if self.port == 0 { String::new() } else { format!(":{}", self.port) };
        let mut parts: Vec<&str> = self.folder.iter().map(String::as_str).collect();
        parts.extend(path.iter().map(String::as_str));
        if let Some(name) = name {
            parts.push(name);
        }
        let remote = parts.join("/");
        let tail = if name.is_some() || remote.is_empty() { "" } else { "/" };
        format!("sftp://{account}{}{port}/{remote}{tail}", self.host)
    }

    /// A session: connected, the key checked against the one trusted, signed in, the SFTP
    /// subsystem up. A key not trusted, or another than the trusted one, ends it before the
    /// password goes out, with the key the server showed.
    async fn session(&self) -> Result<Session, Error> {
        let trust = Arc::new(Trust { known: self.known_key.clone(), shown: Mutex::new(None), changed: AtomicBool::new(false), unknown: AtomicBool::new(false) });
        let config = Arc::new(client::Config {
            inactivity_timeout: Some(Duration::from_secs(READ_PATIENCE * 2)),
            keepalive_interval: Some(Duration::from_secs(15)),
            ..Default::default()
        });
        let connecting = client::connect(config, (self.host.as_str(), self.port()), Handler { trust: trust.clone() });
        let mut handle = match tokio::time::timeout(CONNECT_PATIENCE, connecting).await {
            Ok(Ok(handle)) => handle,
            Ok(Err(err)) => {
                let shown = trust.shown.lock().unwrap().clone().unwrap_or_default();
                if trust.changed.load(Ordering::Relaxed) {
                    return Err(Error::KeyChanged(shown));
                }
                if trust.unknown.load(Ordering::Relaxed) {
                    return Err(Error::KeyUnknown(shown));
                }
                return Err(ssh_error(err));
            }
            Err(_) => return Err(Error::Unreachable("the server did not answer in time".into())),
        };
        match handle.authenticate_password(self.username.clone(), self.password.clone()).await {
            Ok(AuthResult::Success) => {}
            Ok(AuthResult::Failure { .. }) => return Err(Error::Unauthorized),
            Err(err) => return Err(ssh_error(err)),
        }
        let channel = handle.channel_open_session().await.map_err(ssh_error)?;
        channel.request_subsystem(true, "sftp").await.map_err(ssh_error)?;
        let sftp = SftpSession::new(channel.into_stream()).await.map_err(sftp_error)?;
        sftp.set_timeout(READ_PATIENCE);
        Ok(Session { handle, sftp })
    }

    /// The folder's entries.
    pub fn list(&self, http: &Http, path: &[String]) -> Result<Vec<Entry>, Error> {
        http.block_on(async {
            let session = self.session().await?;
            let listed = session.sftp.read_dir(self.remote_path(path, None)).await.map_err(sftp_error)?;
            let entries = listed
                .filter(|entry| entry.file_name() != "." && entry.file_name() != "..")
                .map(|entry| {
                    let meta = entry.metadata();
                    let is_dir = entry.file_type().is_dir();
                    Entry { name: entry.file_name(), is_dir, size: if is_dir { 0 } else { meta.size.unwrap_or(0) }, modified: meta.mtime.map(i64::from) }
                })
                .collect();
            session.close().await;
            Ok(entries)
        })
    }

    /// Whether the server answers at all with this account.
    pub fn check(&self, http: &Http) -> Result<(), Error> {
        http.block_on(async {
            let session = self.session().await?;
            session.close().await;
            Ok(())
        })
    }

    /// The file into `dest`, under the download folder `root`, in chunks over up to
    /// `streams.count` sessions at once, going on from the chunks a `.part` has.
    #[allow(clippy::too_many_arguments)]
    pub fn download(
        &self,
        http: &Http,
        path: &[String],
        name: &str,
        root: &Path,
        dest: &Path,
        streams: &transfer::Streams,
        cancel: &AtomicBool,
        progress: &(dyn Fn(transfer::Progress) + Sync),
    ) -> Result<PathBuf, transfer::Error> {
        transfer::make_folder(root, dest)?;
        let started = Instant::now();
        let remote = self.remote_path(path, Some(name));
        let link = self.link(path, Some(name));
        let part = transfer::with_suffix(dest, ".part");
        let state_path = transfer::with_suffix(dest, ".part.json");
        // The file's size and date, from the first session, which then fetches too.
        let first = http.block_on(self.session()).map_err(super::remote::transfer_error)?;
        let meta = http.block_on(first.sftp.metadata(remote.clone())).map_err(|err| super::remote::transfer_error(sftp_error(err)))?;
        let total = meta.size.unwrap_or(0);
        let modified = format!("{total}-{}", meta.mtime.unwrap_or(0));
        let report = |done: u64| progress(transfer::Progress { done, total, protocol: transfer::Protocol::Unknown, waiting: 0 });
        let file = transfer::open_own(root, &part, false)?;
        let chunks = usize::try_from(total.div_ceil(CHUNK)).unwrap_or(usize::MAX);
        let mut done = transfer::resume_state(&state_path, &link, &modified, total, CHUNK).filter(|_| file.metadata().map(|meta| meta.len() == total).unwrap_or(false)).unwrap_or_default();
        if done.len() != chunks {
            file.set_len(total).map_err(|err| transfer::Error::Disk(err.to_string()))?;
            done = vec![false; chunks];
        }
        let already: u64 = done.iter().enumerate().filter(|(_, done)| **done).map(|(index, _)| chunk_length(index, total)).sum();
        transfer::check_space(dest, total.saturating_sub(already))?;
        report(already);
        let pending: Mutex<Vec<usize>> = Mutex::new((0..chunks).rev().filter(|index| !done[*index]).collect());
        let done = Mutex::new(done);
        let bytes = AtomicU64::new(already);
        let failed: Mutex<Option<transfer::Error>> = Mutex::new(None);
        let reported = Mutex::new(Instant::now());
        let workers = streams.count.min(pending.lock().unwrap().len()).max(1);
        let first = Mutex::new(Some(first));
        std::thread::scope(|scope| {
            for _ in 0..workers {
                let (pending, done, bytes, failed, reported, first, file, report) = (&pending, &done, &bytes, &failed, &reported, &first, &file, &report);
                let (remote, link, modified, state_path) = (remote.clone(), link.clone(), modified.clone(), state_path.clone());
                scope.spawn(move || {
                    let outcome = http.block_on(async {
                        let _permit = streams.acquire().await?;
                        let taken = first.lock().unwrap().take();
                        let session = match taken {
                            Some(session) => session,
                            None => self.session().await.map_err(super::remote::transfer_error)?,
                        };
                        let mut reader = session.sftp.open(remote).await.map_err(|err| super::remote::transfer_error(sftp_error(err)))?;
                        let mut buffer = vec![0u8; PIECE];
                        loop {
                            if cancel.load(Ordering::Relaxed) || failed.lock().unwrap().is_some() {
                                break;
                            }
                            let Some(index) = pending.lock().unwrap().pop() else { break };
                            let start = index as u64 * CHUNK;
                            let length = chunk_length(index, total);
                            reader.seek(std::io::SeekFrom::Start(start)).await.map_err(|err| transfer::Error::Connection(err.to_string()))?;
                            let mut got: u64 = 0;
                            while got < length {
                                if cancel.load(Ordering::Relaxed) {
                                    return Err(transfer::Error::Cancelled);
                                }
                                let want = (length - got).min(PIECE as u64) as usize;
                                let read = reader.read(&mut buffer[..want]).await.map_err(|err| transfer::Error::Connection(err.to_string()))?;
                                if read == 0 {
                                    return Err(transfer::Error::Protocol(format!("the file ended {} bytes short", length - got)));
                                }
                                file.write_at(&buffer[..read], start + got).map_err(|err| transfer::Error::Disk(err.to_string()))?;
                                got += read as u64;
                                let so_far = bytes.fetch_add(read as u64, Ordering::Relaxed) + read as u64;
                                let mut last = reported.lock().unwrap();
                                if last.elapsed() >= Duration::from_millis(50) {
                                    *last = Instant::now();
                                    drop(last);
                                    report(so_far);
                                }
                            }
                            let mut done = done.lock().unwrap();
                            done[index] = true;
                            transfer::write_resume(root, &state_path, &link, &modified, total, CHUNK, &done);
                        }
                        session.close().await;
                        Ok::<(), transfer::Error>(())
                    });
                    if let Err(err) = outcome {
                        let mut failed = failed.lock().unwrap();
                        if failed.is_none() {
                            *failed = Some(err);
                        }
                    }
                });
            }
        });
        if cancel.load(Ordering::Relaxed) {
            return Err(transfer::Error::Cancelled);
        }
        if let Some(err) = failed.lock().unwrap().take() {
            return Err(err);
        }
        file.sync_all().map_err(|err| transfer::Error::Disk(err.to_string()))?;
        drop(file);
        report(total);
        let target = transfer::finish(root, dest)?;
        log::line(format!("done {}: {:.1} MB in {:.1} s over {workers} sessions", log::shown(&link), total as f64 / (1024.0 * 1024.0), started.elapsed().as_secs_f64()));
        Ok(target)
    }
}

/// How long chunk `index` of a file of `total` bytes is.
fn chunk_length(index: usize, total: u64) -> u64 {
    total.saturating_sub(index as u64 * CHUNK).min(CHUNK)
}

/// A connection with its SFTP subsystem.
struct Session {
    handle: client::Handle<Handler>,
    sftp: SftpSession,
}

impl Session {
    async fn close(self) {
        let _ = self.sftp.close().await;
        let _ = self.handle.disconnect(russh::Disconnect::ByApplication, "", "en").await;
    }
}

/// Whether the server's key is the one trusted, and the key it showed: another one (`changed`),
/// or one while none is trusted yet (`unknown`).
struct Trust {
    known: KnownKey,
    shown: Mutex<Option<String>>,
    changed: AtomicBool,
    unknown: AtomicBool,
}

struct Handler {
    trust: Arc<Trust>,
}

impl client::Handler for Handler {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let shown = match key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.to_openssh().unwrap_or_default(),
            PublicKeyOrCertificate::Certificate(certificate) => certificate.to_openssh().unwrap_or_default(),
        };
        *self.trust.shown.lock().unwrap() = Some(shown.clone());
        match &*self.trust.known.lock().unwrap() {
            Some(known) if *known == shown => Ok(true),
            Some(_) => {
                self.trust.changed.store(true, Ordering::Relaxed);
                Ok(false)
            }
            // Not trusted yet: the user is asked first, and nothing more is sent now.
            None => {
                self.trust.unknown.store(true, Ordering::Relaxed);
                Ok(false)
            }
        }
    }
}

/// What an SSH failure means.
fn ssh_error(error: russh::Error) -> Error {
    match error {
        russh::Error::IO(err) => Error::Unreachable(err.to_string()),
        russh::Error::ConnectionTimeout | russh::Error::InactivityTimeout => Error::Unreachable("the server did not answer in time".into()),
        russh::Error::UnknownKey => Error::KeyUnknown(String::new()),
        russh::Error::NotAuthenticated | russh::Error::NoAuthMethod => Error::Unauthorized,
        other => Error::Unreadable(other.to_string()),
    }
}

/// What an SFTP failure means.
fn sftp_error(error: russh_sftp::client::error::Error) -> Error {
    match error {
        russh_sftp::client::error::Error::Status(status) => match status.status_code {
            StatusCode::NoSuchFile => Error::Missing,
            StatusCode::PermissionDenied => Error::Forbidden,
            _ => Error::Unreadable(format!("{}: {}", status.status_code, status.error_message)),
        },
        russh_sftp::client::error::Error::Timeout => Error::Unreachable("the server did not answer in time".into()),
        other => Error::Unreadable(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_told_by_its_kind_and_its_sha256_fingerprint_as_ssh_keygen_prints_them() {
        // ssh-keygen -t ed25519; ssh-keygen -lf: "256 SHA256:d4zw… (ED25519)".
        let line = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAX3y3IcQy0VO2E+SEQHMiZ9vTaYqU6JQqpXbOaPFa1f";
        assert_eq!(key_facts(line), Some(KeyFacts { kind: "ED25519".into(), fingerprint: "SHA256:d4zwBycdXMo0aTCD+wahlolKFXLBphBuPnq52cJYlwo".into() }));
        assert_eq!(key_facts(&format!("{line} comment")).map(|facts| facts.kind), Some("ED25519".into()));
        assert_eq!(key_facts("not a key"), None);
        assert_eq!(key_facts(""), None);
    }
}
