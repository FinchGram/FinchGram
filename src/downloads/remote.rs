//! What the servers of the downloads page have in common, whatever they speak: a protocol with
//! its default port, an address as the user types it, a listing's entries, the ways a server can
//! refuse, and `Remote`, one of the three clients (webdav.rs, ftp.rs, sftp.rs) behind one face
//! for the page (mod.rs): list a folder, check a server, say where a file is, fetch it.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use super::http::{Failure, Http};
use super::{ftp, sftp, transfer, webdav};

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
    /// An SSH server showed another key than the one it was first met with: not the same
    /// server, or someone in between. The key it showed now.
    KeyChanged(String),
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
        }
    }
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

    /// Bytes of `dest` already here from an earlier try of this file: the done chunks of its
    /// `.part` (WebDAV, SFTP), or the `.part` as far as it goes (FTP).
    pub fn kept(&self, dest: &Path, path: &[String], name: &str) -> u64 {
        match self {
            Remote::WebDav(_) | Remote::Sftp(_) => transfer::kept(dest, &self.file_link(path, name)),
            Remote::Ftp(_) => ftp::kept(dest),
        }
    }

    /// The file into `dest`, reporting to `progress` as it goes (transfer::download for what the
    /// words mean). Blocks: call it on a thread of the file's own.
    #[allow(clippy::too_many_arguments)]
    pub fn download(
        &self,
        http: &Http,
        path: &[String],
        name: &str,
        dest: &Path,
        streams: &transfer::Streams,
        cancel: &AtomicBool,
        progress: &(dyn Fn(transfer::Progress) + Sync),
    ) -> Result<PathBuf, transfer::Error> {
        match self {
            Remote::WebDav(server) => {
                let source = transfer::Source { url: server.file_url(path, name), authorization: server.authorization() };
                transfer::download(http, &source, dest, streams, cancel, progress)
            }
            Remote::Ftp(server) => server.download(path, name, dest, cancel, progress),
            Remote::Sftp(server) => server.download(http, path, name, dest, streams, cancel, progress),
        }
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
        Error::KeyChanged(_) => transfer::Error::KeyChanged,
    }
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
    fn a_protocol_has_its_id_and_name() {
        for protocol in [Protocol::Webdav, Protocol::Ftp, Protocol::Ftps, Protocol::Sftp] {
            assert_eq!(Protocol::parse(protocol.id()), Some(protocol));
        }
        assert_eq!(Protocol::parse("gopher"), None);
        assert_eq!(Protocol::Sftp.name(), "SFTP");
    }
}
