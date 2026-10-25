//! FTP and FTPS for the downloads page, over suppaftp: a connection a listing, a connection a
//! file, blocking, on the worker or the download's own thread (remote.rs says who calls what).
//! FTPS is TLS from the first byte on port 990 (implicit), and AUTH TLS on any other port
//! (explicit), with the same trust as the HTTP client's: the Mac's own store. A file comes over
//! one stream (FTP gives a start, never an end: no ranges), REST going on from where a `.part`
//! stopped, when the `.part.json` beside it says the `.part` is this file's, in this version (the
//! WebDAV engine's state file, with no chunks in it). Paths are relative to the folder the
//! account lands in: "host/films" lists "films" there. RETR and LIST (MLSD) read; DELE, RMD,
//! RNFR/RNTO, MKD and STOR change, and only on a server the user allows changes on (the design's
//! ninth round). A folder is deleted by walking it: FTP has no deep delete.

use std::io::{BufRead as _, Read, Seek as _, Write as _};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs as _};
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
/// A folder deleted is walked first: at most this many folders, this deep (as a folder
/// download, src/downloads/mod.rs).
const MOST_DELETED_FOLDERS: usize = 100_000;
const DEEPEST_DELETE: usize = 64;

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

    /// Connected, logged in, in binary: ready for a listing or a file. The connection, the
    /// welcome and the login each wait CONNECT_PATIENCE at most, a read after that READ_PATIENCE:
    /// a server that takes the connection and says nothing does not hold the servers' thread
    /// (src/downloads/mod.rs). A data connection goes to the address the control connection
    /// reached, whatever address PASV names: a server cannot send the client to another machine,
    /// or to the Mac itself.
    fn connect(&self) -> Result<RustlsFtpStream, Error> {
        let socket = (self.host.as_str(), self.port())
            .to_socket_addrs()
            .map_err(|err| Error::Unreachable(err.to_string()))?
            .next()
            .ok_or_else(|| Error::Unreachable(format!("{} is not known", self.host)))?;
        let stream = if self.tls && self.port() == 990 {
            // suppaftp's implicit TLS connects on its own, and waits for the welcome with no
            // timeout: it is given one here, on a thread of its own, which is left to end on its
            // own when the server keeps it waiting.
            let host = self.host.clone();
            with_deadline(CONNECT_PATIENCE + CONNECT_PATIENCE, move || RustlsFtpStream::connect_secure_implicit(socket, tls(), &host))?.map_err(ftp_error)?
        } else {
            let tcp = TcpStream::connect_timeout(&socket, CONNECT_PATIENCE).map_err(|err| Error::Unreachable(err.to_string()))?;
            let _ = tcp.set_read_timeout(Some(CONNECT_PATIENCE));
            let _ = tcp.set_write_timeout(Some(CONNECT_PATIENCE));
            let plain = RustlsFtpStream::connect_with_stream(tcp).map_err(ftp_error)?;
            if self.tls { plain.into_secure(tls(), &self.host).map_err(ftp_error)? } else { plain }
        };
        let peer = socket.ip();
        let mut stream = stream.passive_stream_builder(move |named| data_connection(SocketAddr::new(peer, named.port())));
        let _ = stream.get_ref().set_read_timeout(Some(CONNECT_PATIENCE));
        stream.set_mode(Mode::Passive);
        let (user, password) = if self.username.is_empty() { ("anonymous", "finchgram@") } else { (self.username.as_str(), self.password.as_str()) };
        stream.login(user, password).map_err(ftp_error)?;
        stream.transfer_type(FileType::Binary).map_err(ftp_error)?;
        let _ = stream.get_ref().set_read_timeout(Some(READ_PATIENCE));
        Ok(stream)
    }

    /// The folder's entries: MLSD where the server has it, LIST parsed otherwise.
    pub fn list(&self, path: &[String]) -> Result<Vec<Entry>, Error> {
        let mut stream = self.connect()?;
        let folder = self.remote_path(path, None);
        let entries = entries(&mut stream, &folder);
        let _ = stream.quit();
        entries
    }

    /// The file, or the folder with everything in it, gone: DELE, or the folder walked and
    /// emptied from the bottom up, then RMD.
    pub fn delete(&self, path: &[String], name: &str, is_dir: bool) -> Result<(), Error> {
        let mut stream = self.connect()?;
        let remote = self.remote_path(path, Some(name));
        let outcome = if is_dir { remove_tree(&mut stream, &remote) } else { stream.rm(&remote).map_err(change_error) };
        let _ = stream.quit();
        outcome
    }

    /// `name` in `from` moved to `to` as `new_name`: RNFR and RNTO. What `replace` names at the
    /// destination (a file, `Some(false)`, or a folder, `Some(true)`) goes first; with `None`
    /// the server does as it does with a taken name, which most refuse.
    pub fn move_to(&self, from: &[String], name: &str, to: &[String], new_name: &str, replace: Option<bool>) -> Result<(), Error> {
        let mut stream = self.connect()?;
        let source = self.remote_path(from, Some(name));
        let destination = self.remote_path(to, Some(new_name));
        let outcome = match replace {
            Some(true) => remove_tree(&mut stream, &destination),
            Some(false) => stream.rm(&destination).map_err(change_error),
            None => Ok(()),
        }
        .and_then(|()| stream.rename(&source, &destination).map_err(change_error));
        let _ = stream.quit();
        outcome
    }

    /// A new folder `name` in `path`: MKD.
    pub fn make_folder(&self, path: &[String], name: &str) -> Result<(), Error> {
        let mut stream = self.connect()?;
        let outcome = stream.mkdir(self.remote_path(path, Some(name))).map_err(change_error);
        let _ = stream.quit();
        outcome
    }

    /// The file at `local`, `size` bytes, STORed into `path` as `name` over one stream, from its
    /// first byte; the bytes sent go to `progress` as they go.
    pub fn upload(&self, local: &Path, size: u64, path: &[String], name: &str, cancel: &AtomicBool, progress: &(dyn Fn(u64) + Sync)) -> Result<(), transfer::Error> {
        let mut file = std::fs::File::open(local).map_err(|err| transfer::Error::Disk(err.to_string()))?;
        let mut stream = self.connect().map_err(super::remote::transfer_error)?;
        let remote = self.remote_path(path, Some(name));
        let mut out = stream.put_with_stream(&remote).map_err(|err| super::remote::transfer_error(change_error(err)))?;
        let mut buffer = vec![0u8; PIECE];
        let mut sent: u64 = 0;
        let mut reported = Instant::now();
        while sent < size {
            if cancel.load(Ordering::Relaxed) {
                let _ = stream.abort(out);
                return Err(transfer::Error::Cancelled);
            }
            let want = (size - sent).min(PIECE as u64) as usize;
            let read = match file.read(&mut buffer[..want]) {
                Ok(0) => {
                    let _ = stream.abort(out);
                    return Err(transfer::Error::Disk(format!("the file ended {} bytes early", size - sent)));
                }
                Ok(read) => read,
                Err(err) => {
                    let _ = stream.abort(out);
                    return Err(transfer::Error::Disk(err.to_string()));
                }
            };
            out.write_all(&buffer[..read]).map_err(|err| transfer::Error::Connection(format!("the connection was lost: {err}")))?;
            sent += read as u64;
            if reported.elapsed() >= Duration::from_millis(50) {
                progress(sent);
                reported = Instant::now();
            }
        }
        out.finish().map_err(|err| super::remote::transfer_error(change_error(err)))?;
        let _ = stream.quit();
        progress(sent);
        Ok(())
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
        // The room on the disk: for what is left of the file when the server said its size, and,
        // size or no size, asked about again as the bytes come (below), since a server may send
        // more than it said, or without end.
        transfer::check_space(dest, total.map_or(0, |total| total.saturating_sub(done)))?;
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
            let mut checked = done;
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
                // A server that sends more than the size it gave is not followed past it.
                if let Some(total) = total
                    && done + read as u64 > total
                {
                    return Err(transfer::Error::Protocol(format!("the server sent more than the {total} bytes it said the file has")));
                }
                file.write_all(&buffer[..read]).map_err(|err| transfer::Error::Disk(err.to_string()))?;
                done += read as u64;
                if done - checked >= transfer::SPACE_CHECK_EVERY {
                    transfer::check_space(dest, 0)?;
                    checked = done;
                }
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

/// A data connection, to `target`: the control connection's peer at the port PASV named, with
/// the same patience as the control connection.
fn data_connection(target: SocketAddr) -> Result<TcpStream, FtpError> {
    let data = TcpStream::connect_timeout(&target, CONNECT_PATIENCE).map_err(FtpError::ConnectionError)?;
    let _ = data.set_read_timeout(Some(READ_PATIENCE));
    let _ = data.set_write_timeout(Some(READ_PATIENCE));
    Ok(data)
}

/// `work` on a thread of its own, and its outcome within `patience`; past that the server is
/// unreachable, and the thread is left to end on its own (its socket does, once the system or the
/// server gives it up).
fn with_deadline<T: Send + 'static>(patience: Duration, work: impl FnOnce() -> T + Send + 'static) -> Result<T, Error> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("ftps-connect".into())
        .spawn(move || {
            let _ = sender.send(work());
        })
        .map_err(|err| Error::Unreachable(format!("cannot start a thread to connect: {err}")))?;
    receiver.recv_timeout(patience).map_err(|_| Error::Unreachable("the server did not answer in time".into()))
}

/// The entries of `folder` ("" for the one the account lands in) over `stream`: MLSD where the
/// server has it, LIST parsed otherwise; "." and ".." left out.
fn entries(stream: &mut RustlsFtpStream, folder: &str) -> Result<Vec<Entry>, Error> {
    let where_ = if folder.is_empty() { None } else { Some(folder) };
    let entries: Vec<Entry> = match listing(stream, "MLSD", where_) {
        Ok(lines) => lines.iter().filter_map(|line| ListParser::parse_mlsd(line).ok()).map(entry_of).collect(),
        // A listing beyond the bounds is not asked for again another way.
        Err(Listing::Bounds(why)) => return Err(Error::Unreadable(why)),
        // A server without MLSD says so: LIST, then.
        Err(Listing::Ftp(_)) => {
            let lines = listing(stream, "LIST", where_).map_err(|failure| match failure {
                Listing::Ftp(err) => ftp_error(err),
                Listing::Bounds(why) => Error::Unreadable(why),
            })?;
            lines.iter().filter_map(|line| ListParser::parse_posix(line).or_else(|_| ListParser::parse_dos(line)).ok()).map(entry_of).collect()
        }
    };
    Ok(entries.into_iter().filter(|entry| entry.name != "." && entry.name != "..").collect())
}

/// The folder at `root` and everything in it gone: its folders found first, each listed once,
/// their files deleted as they are met, then the folders removed from the deepest up. A name
/// that is not a name (webdav::is_plain_name) stops it: the server does not get to say what
/// else goes.
fn remove_tree(stream: &mut RustlsFtpStream, root: &str) -> Result<(), Error> {
    let mut folders: Vec<(String, usize)> = vec![(root.to_string(), 0)];
    let mut index = 0;
    while index < folders.len() {
        let (folder, depth) = folders[index].clone();
        index += 1;
        for entry in entries(stream, &folder)? {
            if !super::webdav::is_plain_name(&entry.name) {
                return Err(Error::Unreadable(format!("the server named an entry in a way that is not a name: {:?}", entry.name)));
            }
            let child = format!("{folder}/{}", entry.name);
            if entry.is_dir {
                if depth + 1 > DEEPEST_DELETE {
                    return Err(Error::Unreadable(format!("the folder's folders go more than {DEEPEST_DELETE} deep")));
                }
                if folders.len() >= MOST_DELETED_FOLDERS {
                    return Err(Error::Unreadable(format!("the folder holds more than {MOST_DELETED_FOLDERS} folders")));
                }
                folders.push((child, depth + 1));
            } else {
                stream.rm(&child).map_err(change_error)?;
            }
        }
    }
    for (folder, _) in folders.iter().rev() {
        stream.rmdir(folder).map_err(change_error)?;
    }
    Ok(())
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

/// What an FTP refusal of a change means: the server's own words for a 4xx or 5xx reply
/// ("550 Delete operation failed."), which say more than the code alone; the account's and the
/// connection's failures as for a read.
fn change_error(error: FtpError) -> Error {
    match error {
        FtpError::UnexpectedResponse(response)
            if !matches!(response.status, Status::NotLoggedIn | Status::NeedPassword | Status::InvalidCredentials | Status::LoginNeedAccount | Status::NotAvailable | Status::HostUnavailable)
                && (400..600).contains(&response.status.code()) =>
        {
            let words = String::from_utf8_lossy(&response.body).trim().to_string();
            let code = response.status.code();
            Error::Refused(if words.starts_with(&code.to_string()) { words } else { format!("{code} {words}") })
        }
        other => ftp_error(other),
    }
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
