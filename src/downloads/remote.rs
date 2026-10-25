//! What the servers of the downloads page have in common, whatever they speak: a protocol with
//! its default port, an address as the user types it, a listing's entries, the ways a server can
//! refuse, and `Remote`, one of the three clients (webdav.rs, ftp.rs, sftp.rs) behind one face
//! for the page (mod.rs): list a folder, check a server, say where a file is, fetch it; and, where
//! the user allows changes on the server (the design's ninth round), delete, move, rename, make
//! a folder and upload.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use super::http::{Failure, Http};
use super::{ftp, log, sftp, transfer, webdav};

/// What a server speaks (the design's seventh round): chosen when the server is added.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Webdav,
    Ftp,
    Ftps,
    Sftp,
}

impl Protocol {
    /// From its id, as the form and the database name it.
    pub fn parse(id: &str) -> Option<Protocol> {
        match id {
            "webdav" => Some(Protocol::Webdav),
            "ftp" => Some(Protocol::Ftp),
            "ftps" => Some(Protocol::Ftps),
            "sftp" => Some(Protocol::Sftp),
            _ => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Protocol::Webdav => "webdav",
            Protocol::Ftp => "ftp",
            Protocol::Ftps => "ftps",
            Protocol::Sftp => "sftp",
        }
    }

    /// As the page writes it.
    pub fn name(self) -> &'static str {
        match self {
            Protocol::Webdav => "WebDAV",
            Protocol::Ftp => "FTP",
            Protocol::Ftps => "FTPS",
            Protocol::Sftp => "SFTP",
        }
    }

}

/// Whether a server allows changes, as far as it said (the design's ninth round): WebDAV's
/// OPTIONS answer names the methods the server allows, and one without DELETE, MOVE, PUT or MKCOL
/// is a server that only reads (PikPak with its read-only switch on); FTP and SFTP say nothing,
/// and a change is tried to find out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Changes {
    #[default]
    Unknown,
    Allowed,
    ReadOnly,
}

/// A folder's entry, whatever listed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    /// A file's size in bytes; 0 for a folder.
    pub size: u64,
    /// When it was last modified, in seconds since the epoch; none when the server did not say.
    pub modified: Option<i64>,
}

/// Why a server could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The account was refused (401; FTP's 530; SSH's authentication).
    Unauthorized,
    /// The account may not see this (403; SSH's permission denied).
    Forbidden,
    /// Any other HTTP error.
    Status(u16),
    /// No connection: the host is unknown, refuses, times out, or its certificate is not trusted.
    Unreachable(String),
    /// The server answered something that is not an answer: not a WebDAV listing, not an FTP
    /// reply, not an SFTP packet.
    Unreadable(String),
    /// The file or folder is not there (FTP's 550, SFTP's no such file).
    Missing,
    /// An SSH server showed another key than the one the user trusted: not the same server, or
    /// someone in between. The key it showed now.
    KeyChanged(String),
    /// An SSH server not trusted yet showed its key: the user is asked before the password goes
    /// out. The key it showed.
    KeyUnknown(String),
    /// The server would not make a change, in its own words: an FTP reply ("550 Delete operation
    /// failed."), an SFTP failure's message, a WebDAV Multi-Status.
    Refused(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unauthorized => write!(f, "the server refused the password"),
            Error::Forbidden => write!(f, "the server refused access"),
            Error::Status(code) => write!(f, "the server answered HTTP {code}"),
            Error::Unreachable(why) => write!(f, "cannot reach the server: {why}"),
            Error::Unreadable(why) => write!(f, "the answer is not an answer: {why}"),
            Error::Missing => write!(f, "the server has no such file or folder"),
            Error::KeyChanged(_) => write!(f, "the server's key is not the one it showed before"),
            Error::KeyUnknown(_) => write!(f, "the server's key is not trusted yet"),
            Error::Refused(words) => write!(f, "the server refused: {words}"),
        }
    }
}

impl Error {
    /// Whether the server itself said no: the page words such a failure as a refusal, "The server
    /// refused to delete … (403 Forbidden)", and anything else as what went wrong.
    pub fn is_refusal(&self) -> bool {
        matches!(self, Error::Unauthorized | Error::Forbidden | Error::Status(_) | Error::Missing | Error::Refused(_))
    }

    /// The refusal in a few words, within the page's parentheses: the HTTP status with its
    /// reason, SSH's "Permission denied", an FTP reply as the server gave it.
    pub fn refusal(&self, protocol: Protocol) -> String {
        let (sftp, ftp) = (protocol == Protocol::Sftp, matches!(protocol, Protocol::Ftp | Protocol::Ftps));
        match self {
            Error::Forbidden if sftp => "Permission denied".into(),
            Error::Forbidden if ftp => "550 Permission denied".into(),
            Error::Forbidden => "403 Forbidden".into(),
            Error::Unauthorized if sftp => "the password was refused".into(),
            Error::Unauthorized if ftp => "530 Not logged in".into(),
            Error::Unauthorized => "401 Unauthorized".into(),
            Error::Missing if sftp => "No such file".into(),
            Error::Missing if ftp => "550 No such file".into(),
            Error::Missing => "404 Not Found".into(),
            Error::Status(code) => http_status(*code),
            Error::Refused(words) => words.clone(),
            other => other.to_string(),
        }
    }
}

/// An HTTP status with its reason, "423 Locked", for the ones a WebDAV server answers a change
/// with; "HTTP 599" for the rest.
pub fn http_status(code: u16) -> String {
    let reason = match code {
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        412 => "Precondition Failed",
        413 => "Content Too Large",
        415 => "Unsupported Media Type",
        423 => "Locked",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        507 => "Insufficient Storage",
        _ => return format!("HTTP {code}"),
    };
    format!("{code} {reason}")
}

impl From<Failure> for Error {
    fn from(failure: Failure) -> Self {
        match failure {
            Failure::Connection(why) => Error::Unreachable(why),
            Failure::Timeout => Error::Unreachable("the server did not answer in time".into()),
            Failure::Redirect(why) => Error::Unreadable(why),
        }
    }
}

/// The address without a scheme the user may have typed: "sftp://sb.example.net/films" →
/// "sb.example.net/films". For FTP and SFTP, whose addresses are a host and a folder.
pub fn bare_address(address: &str) -> String {
    let address = address.trim();
    match address.find("://") {
        Some(at) => address[at + 3..].to_string(),
        None => address.to_string(),
    }
    .trim_matches('/')
    .to_string()
}

/// Where an FTP or SFTP address points: the host, and the folder under the account's own: "host",
/// "host/films/2026", "sftp://host/films" (a scheme is let go). A port written after the host
/// ("host:2222") is let go too: the port is the server's own field.
pub fn host_and_folder(address: &str) -> (String, Vec<String>) {
    let bare = bare_address(address);
    let mut parts = bare.split('/').filter(|part| !part.is_empty() && *part != ".");
    let host = parts.next().unwrap_or_default();
    let host = host.rsplit('@').next().unwrap_or(host);
    let host = match host.rfind(':') {
        Some(at) if !host.starts_with('[') && host[at + 1..].chars().all(|c| c.is_ascii_digit()) => &host[..at],
        _ => host,
    };
    (host.to_string(), parts.map(str::to_string).collect())
}

/// A port written after the host, "2222" out of "host:2222/films"; "" for none.
pub fn inline_port(address: &str) -> String {
    let bare = bare_address(address);
    let host = bare.split('/').next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or(host);
    let after_bracket = host.rfind(']').map_or(0, |end| end + 1);
    match host[after_bracket..].rfind(':') {
        Some(at) => {
            let digits = &host[after_bracket + at + 1..];
            if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) { digits.to_string() } else { String::new() }
        }
        None => String::new(),
    }
}

/// True for an address FTP or SFTP can use: a host, with nothing odd in it.
pub fn is_host_address(address: &str) -> bool {
    let (host, folder) = host_and_folder(address);
    !host.is_empty() && !host.chars().any(|c| c.is_whitespace() || c == '\\') && folder.iter().all(|part| part != "..")
}

/// One of the clients, behind one face.
#[derive(Clone)]
pub enum Remote {
    WebDav(webdav::Server),
    Ftp(ftp::Server),
    Sftp(sftp::Server),
}

impl Remote {
    pub fn protocol(&self) -> Protocol {
        match self {
            Remote::WebDav(_) => Protocol::Webdav,
            Remote::Ftp(server) if server.tls => Protocol::Ftps,
            Remote::Ftp(_) => Protocol::Ftp,
            Remote::Sftp(_) => Protocol::Sftp,
        }
    }

    /// The folder's entries. Blocks: call it on a worker thread.
    pub fn list(&self, http: &Http, path: &[String]) -> Result<Vec<Entry>, Error> {
        match self {
            Remote::WebDav(server) => server.list(http, path),
            Remote::Ftp(server) => server.list(path),
            Remote::Sftp(server) => server.list(http, path),
        }
    }

    /// The folder's entries, for a tree walked several folders at a time on the runtime: WebDAV
    /// over its own async client, the others on a thread of the pool.
    pub async fn list_in(&self, http: &'static Http, path: &[String]) -> Result<Vec<Entry>, Error> {
        match self {
            Remote::WebDav(server) => server.list_in(http, path).await,
            other => {
                let remote = other.clone();
                let path = path.to_vec();
                tokio::task::spawn_blocking(move || remote.list(http, &path)).await.map_err(|err| Error::Unreachable(format!("the listing's task failed: {err}")))?
            }
        }
    }

    /// Whether the server answers at all with this account: the root looked at.
    pub fn check(&self, http: &Http) -> Result<(), Error> {
        match self {
            Remote::WebDav(server) => server.check(http),
            Remote::Ftp(server) => server.check(),
            Remote::Sftp(server) => server.check(http),
        }
    }

    /// Whether a file comes in ranges, so that several streams can share it: WebDAV asks the
    /// server about `first_file` at the root, SFTP always does, FTP never.
    pub fn ranges(&self, http: &Http, first_file: &str) -> Option<bool> {
        match self {
            Remote::WebDav(server) => {
                let source = transfer::Source { url: server.file_url(&[], first_file), authorization: server.authorization() };
                transfer::probe(http, &source).ok().map(|probe| probe.ranges)
            }
            Remote::Ftp(_) => Some(false),
            Remote::Sftp(_) => Some(true),
        }
    }

    /// The folder's address, as a link to copy: "https://nas.home:5006/dav/films/",
    /// "sftp://me@sb.example.net/films/".
    pub fn folder_link(&self, path: &[String]) -> String {
        match self {
            Remote::WebDav(server) => server.folder_url(path),
            Remote::Ftp(server) => server.link(path, None),
            Remote::Sftp(server) => server.link(path, None),
        }
    }

    /// The file's address, as a link to copy.
    pub fn file_link(&self, path: &[String], name: &str) -> String {
        match self {
            Remote::WebDav(server) => server.file_url(path, name),
            Remote::Ftp(server) => server.link(path, Some(name)),
            Remote::Sftp(server) => server.link(path, Some(name)),
        }
    }

    /// The same server, whatever account: for the queue, which fetches a file once.
    pub fn same_as(&self, other: &Remote) -> bool {
        self.folder_link(&[]) == other.folder_link(&[])
    }

    /// An SFTP server's host and port: what its key is trusted by.
    pub fn ssh_host(&self) -> Option<(&str, u16)> {
        match self {
            Remote::Sftp(server) => Some((server.host.as_str(), server.port())),
            _ => None,
        }
    }

    /// Bytes of `dest` already here from an earlier try of this file, as its `.part.json` names
    /// it: the done chunks of its `.part` (WebDAV, SFTP), or the `.part` as far as it goes (FTP).
    pub fn kept(&self, dest: &Path, path: &[String], name: &str) -> u64 {
        match self {
            Remote::WebDav(_) | Remote::Sftp(_) => transfer::kept(dest, &self.file_link(path, name)),
            Remote::Ftp(_) => transfer::kept_sequential(dest, &self.file_link(path, name)),
        }
    }

    /// The file into `dest`, under the download folder `root`, reporting to `progress` as it goes
    /// (transfer::download for what the words mean). The file is marked as the download it is
    /// (platform::mark_downloaded: macOS's quarantine, with its address, without the account), so
    /// that the system treats it as one. Blocks: call it on a thread of the file's own.
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
        let target = match self {
            Remote::WebDav(server) => {
                let source = transfer::Source { url: server.file_url(path, name), authorization: server.authorization() };
                transfer::download(http, &source, root, dest, streams, cancel, progress)
            }
            Remote::Ftp(server) => server.download(path, name, root, dest, cancel, progress),
            Remote::Sftp(server) => server.download(http, path, name, root, dest, streams, cancel, progress),
        }?;
        crate::platform::mark_downloaded(&target, Some(&log::shown(&self.file_link(path, name))));
        Ok(target)
    }

    /// Whether the server allows changes, as far as it says (`Changes`). Blocks.
    pub fn changes(&self, http: &Http) -> Changes {
        match self {
            Remote::WebDav(server) => server.changes(http),
            Remote::Ftp(_) | Remote::Sftp(_) => Changes::Unknown,
        }
    }

    /// The file, or the folder with everything in it, gone from the server. Blocks.
    pub fn delete(&self, http: &Http, path: &[String], name: &str, is_dir: bool) -> Result<(), Error> {
        match self {
            Remote::WebDav(server) => server.delete(http, path, name, is_dir),
            Remote::Ftp(server) => server.delete(path, name, is_dir),
            Remote::Sftp(server) => server.delete(http, path, name, is_dir),
        }
    }

    /// `name` in the folder `from` moved to the folder `to` as `new_name` (the same folder and
    /// another name: a rename), on the same server. `replace` says what stands at the
    /// destination already and is to go: a file (`Some(false)`), a folder (`Some(true)`); with
    /// `None` the server is told not to overwrite anything. Blocks.
    #[allow(clippy::too_many_arguments)]
    pub fn move_to(&self, http: &Http, from: &[String], name: &str, is_dir: bool, to: &[String], new_name: &str, replace: Option<bool>) -> Result<(), Error> {
        match self {
            Remote::WebDav(server) => server.move_to(http, from, name, is_dir, to, new_name, replace.is_some()),
            Remote::Ftp(server) => server.move_to(from, name, to, new_name, replace),
            Remote::Sftp(server) => server.move_to(http, from, name, to, new_name, replace),
        }
    }

    /// A new folder `name` in the folder `path`. Blocks.
    pub fn make_folder(&self, http: &Http, path: &[String], name: &str) -> Result<(), Error> {
        match self {
            Remote::WebDav(server) => server.make_folder(http, path, name),
            Remote::Ftp(server) => server.make_folder(path, name),
            Remote::Sftp(server) => server.make_folder(http, path, name),
        }
    }

    /// The file at `local`, `size` bytes, into the folder `path` as `name`, over one stream from
    /// its first byte (no upload goes on where it stopped: a retry starts over), reporting the
    /// bytes sent to `progress` as it goes. What is there under that name is replaced. Blocks:
    /// call it on a thread of the file's own.
    #[allow(clippy::too_many_arguments)]
    pub fn upload(&self, http: &Http, local: &Path, size: u64, path: &[String], name: &str, cancel: &Arc<AtomicBool>, progress: &(dyn Fn(u64) + Sync)) -> Result<(), transfer::Error> {
        let started = std::time::Instant::now();
        match self {
            Remote::WebDav(server) => server.upload(http, local, size, path, name, cancel, progress),
            Remote::Ftp(server) => server.upload(local, size, path, name, cancel, progress),
            Remote::Sftp(server) => server.upload(http, local, size, path, name, cancel, progress),
        }?;
        log::line(format!("uploaded {}: {:.1} MB in {:.1} s", log::shown(&self.file_link(path, name)), size as f64 / (1024.0 * 1024.0), started.elapsed().as_secs_f64()));
        Ok(())
    }
}

/// A server's refusal, as the transfer engine words it.
pub fn transfer_error(error: Error) -> transfer::Error {
    match error {
        Error::Unauthorized => transfer::Error::Unauthorized,
        Error::Forbidden => transfer::Error::Forbidden,
        Error::Status(code) => transfer::Error::Status(code),
        Error::Unreachable(why) => transfer::Error::Connection(why),
        Error::Unreadable(why) => transfer::Error::Protocol(why),
        Error::Missing => transfer::Error::Missing,
        Error::KeyChanged(shown) => transfer::Error::KeyChanged(shown),
        Error::KeyUnknown(shown) => transfer::Error::KeyUnknown(shown),
        Error::Refused(words) => transfer::Error::Refused(words),
    }
}

/// A transfer's failure as the servers word theirs: for an upload's row, which says what the
/// server refused in the server's words (`Error::refusal`). None for what is not the server's
/// doing: the user stopped it, or the disk.
pub fn remote_error(error: &transfer::Error) -> Option<Error> {
    Some(match error {
        transfer::Error::Unauthorized => Error::Unauthorized,
        transfer::Error::Forbidden => Error::Forbidden,
        transfer::Error::Status(code) => Error::Status(*code),
        transfer::Error::Missing => Error::Missing,
        transfer::Error::Refused(words) => Error::Refused(words.clone()),
        transfer::Error::Connection(why) => Error::Unreachable(why.clone()),
        transfer::Error::Protocol(why) => Error::Unreadable(why.clone()),
        transfer::Error::KeyChanged(shown) => Error::KeyChanged(shown.clone()),
        transfer::Error::KeyUnknown(shown) => Error::KeyUnknown(shown.clone()),
        transfer::Error::Cancelled | transfer::Error::Disk(_) => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_a_host_and_a_folder() {
        assert_eq!(host_and_folder("sb-ams.example.net"), ("sb-ams.example.net".to_string(), vec![]));
        assert_eq!(host_and_folder(" sftp://me@sb.example.net:2222/films/2026/ "), ("sb.example.net".to_string(), vec!["films".to_string(), "2026".to_string()]));
        assert_eq!(host_and_folder("ftp.example.net//downloads"), ("ftp.example.net".to_string(), vec!["downloads".to_string()]));
        assert_eq!(host_and_folder("[fe80::1]/x"), ("[fe80::1]".to_string(), vec!["x".to_string()]));
        assert_eq!(bare_address("ftps://host/a/"), "host/a");
        assert_eq!(inline_port("host:2222/films"), "2222");
        assert_eq!(inline_port("sftp://me@host:22"), "22");
        assert_eq!(inline_port("host/films"), "");
        assert_eq!(inline_port("[fe80::1]:8022/x"), "8022");
        assert_eq!(inline_port("[fe80::1]/x"), "");
        assert!(is_host_address("sb.example.net/films"));
        assert!(!is_host_address(""));
        assert!(!is_host_address("sftp://"));
        assert!(!is_host_address("a host"));
        assert!(!is_host_address("host/../etc"));
    }

    #[test]
    fn a_refusal_is_worded_as_the_server_would() {
        assert_eq!(Error::Forbidden.refusal(Protocol::Webdav), "403 Forbidden");
        assert_eq!(Error::Forbidden.refusal(Protocol::Sftp), "Permission denied");
        assert_eq!(Error::Forbidden.refusal(Protocol::Ftps), "550 Permission denied");
        assert_eq!(Error::Status(423).refusal(Protocol::Webdav), "423 Locked");
        assert_eq!(Error::Status(599).refusal(Protocol::Webdav), "HTTP 599");
        assert_eq!(Error::Refused("550 Delete operation failed.".into()).refusal(Protocol::Ftp), "550 Delete operation failed.");
        assert_eq!(Error::Missing.refusal(Protocol::Webdav), "404 Not Found");
        assert!(Error::Status(507).is_refusal());
        assert!(!Error::Unreachable("x".into()).is_refusal());
        assert!(Error::Unreachable("no route".into()).refusal(Protocol::Webdav).contains("no route"));
    }

    #[test]
    fn a_protocol_has_its_id_and_name() {
        for protocol in [Protocol::Webdav, Protocol::Ftp, Protocol::Ftps, Protocol::Sftp] {
            assert_eq!(Protocol::parse(protocol.id()), Some(protocol));
        }
        assert_eq!(Protocol::parse("gopher"), None);
        assert_eq!(Protocol::Sftp.name(), "SFTP");
    }
}
