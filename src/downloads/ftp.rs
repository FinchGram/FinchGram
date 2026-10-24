//! FTP and FTPS for the downloads page, over suppaftp: a connection a listing, a connection a
//! file, blocking, on the worker or the download's own thread (remote.rs says who calls what).
//! FTPS is TLS from the first byte on port 990 (implicit), and AUTH TLS on any other port
//! (explicit), with the same trust as the HTTP client's: the Mac's own store. A file comes over
//! one stream (FTP gives a start, never an end: no ranges), REST going on from where a `.part`
//! stopped. Paths are relative to the folder the account lands in: "host/films" lists "films"
//! there. FinchGram only reads: RETR and LIST, nothing else.

use std::fs;
use std::io::{Read, Seek as _, Write as _};
use std::net::ToSocketAddrs as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use rustls_platform_verifier::ConfigVerifierExt as _;
use suppaftp::list::ListParser;
use suppaftp::types::FileType;
use suppaftp::{FtpError, Mode, RustlsConnector, RustlsFtpStream, Status};

use super::remote::{Entry, Error};
use super::{log, transfer};

/// How long the TCP connection may take, and how long a read of the control or data connection
/// may bring nothing.
const CONNECT_PATIENCE: Duration = Duration::from_secs(20);
const READ_PATIENCE: Duration = Duration::from_secs(60);
/// A piece of the file read at a time.
const PIECE: usize = 256 * 1024;

#[derive(Debug, Clone)]
pub struct Server {
    pub host: String,
    /// 0 for the default: 21, or 990 with TLS.
    pub port: u16,
    /// The folder under the one the account lands in; none for that one.
    pub folder: Vec<String>,
    /// FTPS.
    pub tls: bool,
    pub username: String,
    pub password: String,
}

impl Server {
    pub fn port(&self) -> u16 {
        match self.port {
            0 if self.tls => 990,
            0 => 21,
            port => port,
        }
    }

    /// The path on the server of `path` under the folder, and of `name` in it.
    fn remote_path(&self, path: &[String], name: Option<&str>) -> String {
        let mut parts: Vec<&str> = self.folder.iter().map(String::as_str).collect();
        parts.extend(path.iter().map(String::as_str));
        if let Some(name) = name {
            parts.push(name);
        }
        parts.join("/")
    }

    /// The address as a link to copy: "ftps://me@host:990/films/2026/" or ".../film.mkv".
    pub fn link(&self, path: &[String], name: Option<&str>) -> String {
        let scheme = if self.tls { "ftps" } else { "ftp" };
        let account = if self.username.is_empty() { String::new() } else { format!("{}@", self.username) };
        let port = if self.port == 0 { String::new() } else { format!(":{}", self.port) };
        let remote = self.remote_path(path, name);
        let tail = if name.is_some() || remote.is_empty() { "" } else { "/" };
        format!("{scheme}://{account}{}{port}/{remote}{tail}", self.host)
    }

    /// Connected, logged in, in binary: ready for a listing or a file.
    fn connect(&self) -> Result<RustlsFtpStream, Error> {
        let address = (self.host.as_str(), self.port());
        let mut stream = if self.tls && self.port() == 990 {
            RustlsFtpStream::connect_secure_implicit(address, tls(), &self.host).map_err(ftp_error)?
        } else {
            let socket = address.to_socket_addrs().map_err(|err| Error::Unreachable(err.to_string()))?.next().ok_or_else(|| Error::Unreachable(format!("{} is not known", self.host)))?;
            let plain = RustlsFtpStream::connect_timeout(socket, CONNECT_PATIENCE).map_err(ftp_error)?;
            if self.tls { plain.into_secure(tls(), &self.host).map_err(ftp_error)? } else { plain }
        };
        let _ = stream.get_ref().set_read_timeout(Some(READ_PATIENCE));
        stream.set_mode(Mode::Passive);
        let (user, password) = if self.username.is_empty() { ("anonymous", "finchgram@") } else { (self.username.as_str(), self.password.as_str()) };
        stream.login(user, password).map_err(ftp_error)?;
        stream.transfer_type(FileType::Binary).map_err(ftp_error)?;
        Ok(stream)
    }

    /// The folder's entries: MLSD where the server has it, LIST parsed otherwise.
    pub fn list(&self, path: &[String]) -> Result<Vec<Entry>, Error> {
        let mut stream = self.connect()?;
        let folder = self.remote_path(path, None);
        let where_ = if folder.is_empty() { None } else { Some(folder.as_str()) };
        let entries = match stream.mlsd(where_) {
            Ok(lines) => lines.iter().filter_map(|line| ListParser::parse_mlsd(line).ok()).map(entry_of).collect(),
            Err(_) => {
                let lines = stream.list(where_).map_err(ftp_error)?;
                lines.iter().filter_map(|line| ListParser::parse_posix(line).or_else(|_| ListParser::parse_dos(line)).ok()).map(entry_of).collect::<Vec<Entry>>()
            }
        };
        let _ = stream.quit();
        Ok(entries.into_iter().filter(|entry| entry.name != "." && entry.name != "..").collect())
    }

    /// Whether the server answers at all with this account.
    pub fn check(&self) -> Result<(), Error> {
        let mut stream = self.connect()?;
        let _ = stream.quit();
        Ok(())
    }

    /// The file into `dest` over one stream, going on from where its `.part` stopped.
    pub fn download(&self, path: &[String], name: &str, dest: &Path, cancel: &AtomicBool, progress: &(dyn Fn(transfer::Progress) + Sync)) -> Result<PathBuf, transfer::Error> {
        if let Some(folder) = dest.parent() {
            fs::create_dir_all(folder).map_err(|err| transfer::Error::Disk(err.to_string()))?;
        }
        let started = Instant::now();
        let remote = self.remote_path(path, Some(name));
        let link = self.link(path, Some(name));
        let mut stream = self.connect().map_err(super::remote::transfer_error)?;
        let total = stream.size(&remote).ok().map(|size| size as u64);
        let part = transfer::with_suffix(dest, ".part");
        let mut file = transfer::open_own(&part, false)?;
        let mut done = file.metadata().map_err(|err| transfer::Error::Disk(err.to_string()))?.len();
        if let Some(total) = total {
            if done > total {
                file.set_len(0).map_err(|err| transfer::Error::Disk(err.to_string()))?;
                done = 0;
            }
            transfer::check_space(dest, total.saturating_sub(done))?;
        }
        file.seek(std::io::SeekFrom::End(0)).map_err(|err| transfer::Error::Disk(err.to_string()))?;
        let report = |done: u64| progress(transfer::Progress { done, total: total.unwrap_or(0), protocol: transfer::Protocol::Unknown, waiting: 0 });
        report(done);
        if total != Some(done) {
            if done > 0 {
                stream.resume_transfer(done as usize).map_err(|err| super::remote::transfer_error(ftp_error(err)))?;
            }
            let mut body = stream.retr_as_stream(&remote).map_err(|err| super::remote::transfer_error(ftp_error(err)))?;
            let mut buffer = vec![0u8; PIECE];
            let mut reported = Instant::now();
            loop {
                if cancel.load(Ordering::Relaxed) {
                    drop(body);
                    return Err(transfer::Error::Cancelled);
                }
                let read = match body.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => read,
                    Err(err) => return Err(transfer::Error::Connection(format!("the connection was lost: {err}"))),
                };
                file.write_all(&buffer[..read]).map_err(|err| transfer::Error::Disk(err.to_string()))?;
                done += read as u64;
                if reported.elapsed() >= Duration::from_millis(50) {
                    report(done);
                    reported = Instant::now();
                }
            }
            body.finish().map_err(|err| super::remote::transfer_error(ftp_error(err)))?;
        }
        let _ = stream.quit();
        if let Some(total) = total
            && done != total
        {
            return Err(transfer::Error::Protocol(format!("the server sent {done} bytes of {total}")));
        }
        file.sync_all().map_err(|err| transfer::Error::Disk(err.to_string()))?;
        drop(file);
        report(done);
        let target = transfer::free_path(dest);
        fs::rename(&part, &target).map_err(|err| transfer::Error::Disk(err.to_string()))?;
        log::line(format!("done {}: {:.1} MB in {:.1} s", log::shown(&link), done as f64 / (1024.0 * 1024.0), started.elapsed().as_secs_f64()));
        Ok(target)
    }
}

/// Bytes of `dest` already here from an earlier try: its `.part`, as far as it goes.
pub fn kept(dest: &Path) -> u64 {
    fs::metadata(transfer::with_suffix(dest, ".part")).map(|meta| meta.len()).unwrap_or(0)
}

/// TLS as the HTTP client has it: the Mac's own trust store.
fn tls() -> RustlsConnector {
    static CONFIG: std::sync::OnceLock<Arc<rustls::ClientConfig>> = std::sync::OnceLock::new();
    let config = CONFIG.get_or_init(|| {
        let config = rustls::ClientConfig::with_platform_verifier().unwrap_or_else(|_| {
            rustls::ClientConfig::builder().with_root_certificates(rustls::RootCertStore::empty()).with_no_client_auth()
        });
        Arc::new(config)
    });
    RustlsConnector::from(config.clone())
}

fn entry_of(file: suppaftp::list::File) -> Entry {
    let modified = file.modified().duration_since(std::time::UNIX_EPOCH).ok().map(|since| since.as_secs() as i64);
    Entry { name: file.name().to_string(), is_dir: file.is_directory(), size: if file.is_directory() { 0 } else { file.size() as u64 }, modified }
}

/// What an FTP refusal means.
fn ftp_error(error: FtpError) -> Error {
    match error {
        FtpError::ConnectionError(err) => Error::Unreachable(err.to_string()),
        FtpError::SecureError(why) => Error::Unreachable(why),
        FtpError::UnexpectedResponse(response) => match response.status {
            Status::NotLoggedIn | Status::NeedPassword | Status::InvalidCredentials | Status::LoginNeedAccount => Error::Unauthorized,
            Status::FileUnavailable => Error::Missing,
            Status::NotAvailable | Status::HostUnavailable => Error::Unreachable(String::from_utf8_lossy(&response.body).trim().to_string()),
            status => Error::Unreadable(format!("{} ({})", String::from_utf8_lossy(&response.body).trim(), status as u32)),
        },
        other => Error::Unreadable(other.to_string()),
    }
}
