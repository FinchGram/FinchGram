//! FTP and FTPS for the downloads page, over suppaftp: a connection a listing, a connection a
//! file, blocking, on the worker or the download's own thread (remote.rs says who calls what).
//! FTPS is TLS from the first byte on port 990 (implicit), and AUTH TLS on any other port
//! (explicit), with the same trust as the HTTP client's: the Mac's own store. A file comes over
//! one stream (FTP gives a start, never an end: no ranges), REST going on from where a `.part`
//! stopped, when the `.part.json` beside it says the `.part` is this file's, in this version (the
//! WebDAV engine's state file, with no chunks in it). Paths are relative to the folder the
//! account lands in: "host/films" lists "films" there. FinchGram only reads: RETR and LIST,
//! nothing else.

use std::io::{BufRead as _, Read, Seek as _, Write as _};
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
/// How much of a listing is taken: this many bytes in all (as much as a WebDAV listing may be,
/// webdav.rs), a line this long, this many entries. A server could otherwise send a listing that
/// never ends, and the app would keep it all.
const LARGEST_LISTING: usize = 32 * 1024 * 1024;
const LONGEST_LINE: usize = 64 * 1024;
const MOST_ENTRIES: usize = 200_000;

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
        let entries: Vec<Entry> = match listing(&mut stream, "MLSD", where_) {
            Ok(lines) => lines.iter().filter_map(|line| ListParser::parse_mlsd(line).ok()).map(entry_of).collect(),
            // A listing beyond the bounds is not asked for again another way.
            Err(Listing::Bounds(why)) => return Err(Error::Unreadable(why)),
            // A server without MLSD says so: LIST, then.
            Err(Listing::Ftp(_)) => {
                let lines = listing(&mut stream, "LIST", where_).map_err(|failure| match failure {
                    Listing::Ftp(err) => ftp_error(err),
                    Listing::Bounds(why) => Error::Unreadable(why),
                })?;
                lines.iter().filter_map(|line| ListParser::parse_posix(line).or_else(|_| ListParser::parse_dos(line)).ok()).map(entry_of).collect()
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

    /// The file into `dest`, under the download folder `root`, over one stream, going on from
    /// where its `.part` stopped when that `.part` is this file's.
    pub fn download(&self, path: &[String], name: &str, root: &Path, dest: &Path, cancel: &AtomicBool, progress: &(dyn Fn(transfer::Progress) + Sync)) -> Result<PathBuf, transfer::Error> {
        transfer::make_folder(root, dest)?;
        let started = Instant::now();
        let remote = self.remote_path(path, Some(name));
        let link = self.link(path, Some(name));
        let mut stream = self.connect().map_err(super::remote::transfer_error)?;
        let total = stream.size(&remote).ok().map(|size| size as u64);
        // The file's version, as far as the server tells it: its size, and when it was last
        // changed (MDTM, where the server has it). A `.part` of another version is not gone on
        // from.
        let modified = format!("{}-{}", total.map(|total| total.to_string()).unwrap_or_default(), stream.mdtm(&remote).map(|time| time.to_string()).unwrap_or_default());
        let part = transfer::with_suffix(dest, ".part");
        let state_path = transfer::with_suffix(dest, ".part.json");
        let mut file = transfer::open_own(root, &part, false)?;
        let mut done = file.metadata().map_err(|err| transfer::Error::Disk(err.to_string()))?.len();
        // An earlier try's bytes are gone on from only when its `.part.json` names this file at
        // this address, in this version, and the `.part` is no longer than the file; anything
        // else (a `.part` left by another file of the same name, or put there by someone) starts
        // afresh, from the first byte.
        if !transfer::resume_sequential(&state_path, &link, &modified) || total.is_some_and(|total| done > total) {
            file.set_len(0).map_err(|err| transfer::Error::Disk(err.to_string()))?;
            done = 0;
        }
        transfer::write_sequential(root, &state_path, &link, &modified, total);
        if let Some(total) = total {
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
            // The data connection waits no longer for a byte than the control connection does.
            let _ = body.get_ref().get_ref().set_read_timeout(Some(READ_PATIENCE));
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
        let target = transfer::finish(root, dest)?;
        log::line(format!("done {}: {:.1} MB in {:.1} s", log::shown(&link), done as f64 / (1024.0 * 1024.0), started.elapsed().as_secs_f64()));
        Ok(target)
    }
}

/// Why a listing did not come: the server's refusal, or the listing beyond the bounds.
enum Listing {
    Ftp(FtpError),
    Bounds(String),
}

/// The lines of `command` ("MLSD", "LIST") on `folder` (the account's own for none), read from
/// the data connection as they come and held to the bounds (LARGEST_LISTING, LONGEST_LINE,
/// MOST_ENTRIES), in place of suppaftp's own listing, which keeps whatever the server sends: a
/// server that never ends its listing, or a line of it, would fill the memory.
fn listing(stream: &mut RustlsFtpStream, command: &str, folder: Option<&str>) -> Result<Vec<String>, Listing> {
    let command = match folder {
        Some(folder) => format!("{command} {folder}"),
        None => command.to_string(),
    };
    let (_, data) = stream.custom_data_command(command, &[Status::AboutToSend, Status::AlreadyOpen]).map_err(Listing::Ftp)?;
    let _ = data.get_ref().get_ref().set_read_timeout(Some(READ_PATIENCE));
    let mut reader = std::io::BufReader::new(data);
    let mut lines = Vec::new();
    let mut bytes = 0usize;
    let mut line = Vec::new();
    let beyond = loop {
        line.clear();
        let read = match (&mut reader).take(LONGEST_LINE as u64 + 1).read_until(b'\n', &mut line) {
            Ok(0) => break None,
            Ok(read) => read,
            Err(err) => return Err(Listing::Ftp(FtpError::ConnectionError(err))),
        };
        if line.len() > LONGEST_LINE {
            break Some(format!("a line of the listing is longer than {} KB", LONGEST_LINE / 1024));
        }
        bytes += read;
        if bytes > LARGEST_LISTING {
            break Some(format!("the listing is longer than {} MB", LARGEST_LISTING / (1024 * 1024)));
        }
        if lines.len() >= MOST_ENTRIES {
            break Some(format!("the listing has more than {MOST_ENTRIES} entries"));
        }
        let text = String::from_utf8_lossy(&line);
        let text = text.trim_end_matches(['\r', '\n']);
        if !text.is_empty() {
            lines.push(text.to_string());
        }
    };
    match beyond {
        None => {
            reader.into_inner().finish().map_err(Listing::Ftp)?;
            Ok(lines)
        }
        Some(why) => {
            let _ = stream.abort(reader.into_inner());
            Err(Listing::Bounds(why))
        }
    }
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
