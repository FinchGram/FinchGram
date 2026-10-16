//! The WebDAV client (RFC 4918), read-only: what a folder holds (PROPFIND, depth 1), whether a
//! server answers at all (depth 0), and the address of a file for the transfer engine
//! (transfer.rs). Basic authentication over whatever the address says, http or https; the
//! password comes from the Keychain (src/platform/). Nothing is ever written to the server.

use std::fmt;

use base64::Engine;
use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::Method;
use hyper::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};

use super::http::{Failure, Http};

/// A server as the page knows it: its address without a trailing slash, and the account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    pub url: String,
    pub username: String,
    pub password: String,
}

/// A folder's entry, as PROPFIND describes it.
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
    /// 401: the account was refused.
    Unauthorized,
    /// 403: the account may not see this.
    Forbidden,
    /// Any other HTTP error.
    Status(u16),
    /// No connection: the host is unknown, refuses, times out, or its certificate is not trusted.
    Unreachable(String),
    /// The server answered something that is not a WebDAV answer.
    Unreadable(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unauthorized => write!(f, "the server refused the password (401)"),
            Error::Forbidden => write!(f, "the server refused access (403)"),
            Error::Status(code) => write!(f, "the server answered HTTP {code}"),
            Error::Unreachable(why) => write!(f, "cannot reach the server: {why}"),
            Error::Unreadable(why) => write!(f, "the answer is not a WebDAV answer: {why}"),
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

/// A folder's listing larger than this is not read: it would be thousands of entries.
const LARGEST_LISTING: usize = 32 * 1024 * 1024;

/// What a listing asks for: whether an entry is a folder, its size, when it changed, its name.
const PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<D:propfind xmlns:D="DAV:"><D:prop><D:resourcetype/><D:getcontentlength/><D:getlastmodified/><D:displayname/></D:prop></D:propfind>"#;

impl Server {
    /// The address as it is kept: trimmed, without the trailing slashes.
    pub fn normalized_url(url: &str) -> String {
        url.trim().trim_end_matches('/').to_string()
    }

    /// True for an address that can be asked: http:// or https:// and something after it.
    pub fn is_address(url: &str) -> bool {
        let lower = url.trim().to_ascii_lowercase();
        ["http://", "https://"].iter().any(|scheme| lower.starts_with(scheme) && lower.len() > scheme.len())
    }

    /// The host of an address, for the page: "nas.home" out of "https://nas.home:5006/dav".
    pub fn host(url: &str) -> String {
        let rest = url.trim().split("://").nth(1).unwrap_or(url.trim());
        let host = rest.split(['/', '?', '#']).next().unwrap_or("");
        // "user@host" is not expected, but harmless; the port goes.
        let host = host.rsplit('@').next().unwrap_or(host);
        if host.starts_with('[') {
            // An IPv6 address in brackets keeps its brackets, loses its port.
            match host.find(']') {
                Some(end) => host[..=end].to_string(),
                None => host.to_string(),
            }
        } else {
            host.split(':').next().unwrap_or(host).to_string()
        }
    }

    /// The address of a folder, with the trailing slash a collection has.
    pub fn folder_url(&self, path: &[String]) -> String {
        let mut url = self.url.clone();
        for segment in path {
            url.push('/');
            url.push_str(&encode_segment(segment));
        }
        url.push('/');
        url
    }

    /// The address of a file in a folder.
    pub fn file_url(&self, path: &[String], name: &str) -> String {
        let mut url = self.url.clone();
        for segment in path {
            url.push('/');
            url.push_str(&encode_segment(segment));
        }
        url.push('/');
        url.push_str(&encode_segment(name));
        url
    }

    /// The Authorization header for the account; none without a user name.
    pub fn authorization(&self) -> Option<String> {
        if self.username.is_empty() {
            return None;
        }
        let credentials = base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", self.username, self.password));
        Some(format!("Basic {credentials}"))
    }

    /// What a folder holds: folders and files, as the server lists them (unsorted); the folder
    /// itself is left out.
    pub fn list(&self, http: &Http, path: &[String]) -> Result<Vec<Entry>, Error> {
        http.block_on(self.list_in(http, path))
    }

    /// The same, from inside the runtime (a tree is walked several folders at a time).
    pub async fn list_in(&self, http: &Http, path: &[String]) -> Result<Vec<Entry>, Error> {
        let url = self.folder_url(path);
        let started = std::time::Instant::now();
        let body = self.propfind(http, &url, "1").await?;
        let entries = parse_listing(&body, &url_path(&url))?;
        super::log::line(format!("listed {url}: {} entries in {} ms", entries.len(), started.elapsed().as_millis()));
        Ok(entries)
    }

    /// Whether the server answers at the root: nothing is read but the fact.
    pub fn check(&self, http: &Http) -> Result<(), Error> {
        http.block_on(self.propfind(http, &self.folder_url(&[]), "0")).map(|_| ())
    }

    async fn propfind(&self, http: &Http, url: &str, depth: &str) -> Result<String, Error> {
        let mut request = http
            .request(Method::from_bytes(b"PROPFIND").expect("a method"), url)
            .header("Depth", depth)
            .header(CONTENT_TYPE, "application/xml; charset=utf-8")
            .header(ACCEPT, "application/xml, text/xml");
        if let Some(authorization) = self.authorization() {
            request = request.header(AUTHORIZATION, authorization);
        }
        let request = request.body(Full::new(Bytes::from_static(PROPFIND_BODY.as_bytes()))).map_err(|err| Error::Unreadable(err.to_string()))?;
        let response = http.send(request).await?;
        match response.status().as_u16() {
            207 => {}
            401 => return Err(Error::Unauthorized),
            403 => return Err(Error::Forbidden),
            code if (400..600).contains(&code) => return Err(Error::Status(code)),
            code => return Err(Error::Unreadable(format!("HTTP {code} instead of a Multi-Status answer"))),
        }
        let bytes = Limited::new(response.into_body(), LARGEST_LISTING)
            .collect()
            .await
            .map_err(|err| Error::Unreadable(err.to_string()))?
            .to_bytes();
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

/// The path of an address, decoded, without a trailing slash: "/dav/Media" out of
/// "https://nas.home:5006/dav/Media/". Only "/" for the root.
pub fn url_path(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let path = match rest.find('/') {
        Some(slash) => &rest[slash..],
        None => "/",
    };
    let path = path.split(['?', '#']).next().unwrap_or("/");
    let decoded = decode(path);
    let trimmed = decoded.trim_end_matches('/');
    if trimmed.is_empty() { "/".to_string() } else { trimmed.to_string() }
}

/// A folder's or file's name as part of an address: everything but the characters an address
/// path may hold as they are (RFC 3986's unreserved and sub-delims, ":" and "@") is percent-encoded.
pub fn encode_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        let keep = byte.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:@".contains(&byte);
        if keep {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Percent-decoding; bytes that are not UTF-8 become the replacement character.
pub fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(value) = std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(value);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The entries of a Multi-Status answer to a PROPFIND of depth 1 on `folder_path` (decoded, no
/// trailing slash): every response but the folder's own.
pub fn parse_listing(xml: &str, folder_path: &str) -> Result<Vec<Entry>, Error> {
    let document = roxmltree::Document::parse(xml).map_err(|err| Error::Unreadable(err.to_string()))?;
    let dav = "DAV:";
    let root = document.root_element();
    if !root.has_tag_name((dav, "multistatus")) {
        return Err(Error::Unreadable(format!("<{}> instead of <multistatus>", root.tag_name().name())));
    }
    let mut entries = Vec::new();
    for response in root.children().filter(|node| node.has_tag_name((dav, "response"))) {
        let Some(href) = response.children().find(|node| node.has_tag_name((dav, "href"))).and_then(|node| node.text()) else {
            continue;
        };
        let href_path = url_path(href.trim());
        if href_path == folder_path || href_path == "/" && folder_path == "/" {
            continue;
        }
        // The properties the server could give (status 200), or the first set when it says no status.
        let prop = response
            .children()
            .filter(|node| node.has_tag_name((dav, "propstat")))
            .filter(|propstat| {
                propstat
                    .children()
                    .find(|node| node.has_tag_name((dav, "status")))
                    .and_then(|node| node.text())
                    .is_none_or(|status| status.contains(" 200 "))
            })
            .find_map(|propstat| propstat.children().find(|node| node.has_tag_name((dav, "prop"))));
        let text_of = |name: &str| prop.and_then(|prop| prop.children().find(|node| node.has_tag_name((dav, name)))).and_then(|node| node.text()).map(str::trim);
        let is_dir = prop
            .and_then(|prop| prop.children().find(|node| node.has_tag_name((dav, "resourcetype"))))
            .is_some_and(|kind| kind.children().any(|node| node.has_tag_name((dav, "collection"))))
            || (href.trim().ends_with('/') && prop.is_none());
        let name = href_path.rsplit('/').next().filter(|name| !name.is_empty()).map(str::to_string).or_else(|| text_of("displayname").map(str::to_string));
        let Some(name) = name else { continue };
        let size = if is_dir { 0 } else { text_of("getcontentlength").and_then(|size| size.parse().ok()).unwrap_or(0) };
        let modified = text_of("getlastmodified").and_then(|date| chrono::DateTime::parse_from_rfc2822(date).ok()).map(|date| date.timestamp());
        entries.push(Entry { name, is_dir, size, modified });
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_of_an_address() {
        assert_eq!(Server::host("https://nas.home:5006/dav"), "nas.home");
        assert_eq!(Server::host("http://192.168.1.10:5005"), "192.168.1.10");
        assert_eq!(Server::host("http://[fe80::1]:8080/dav/"), "[fe80::1]");
        assert_eq!(Server::host("http://me@nas.home/"), "nas.home");
        assert_eq!(Server::host(" nas.home "), "nas.home");
    }

    #[test]
    fn an_address_needs_a_scheme_and_a_host() {
        assert!(Server::is_address("http://nas.home"));
        assert!(Server::is_address(" HTTPS://nas.home:5006/dav/ "));
        assert!(!Server::is_address("https://"));
        assert!(!Server::is_address("ftp://nas.home"));
        assert!(!Server::is_address("nas.home"));
        assert_eq!(Server::normalized_url(" https://nas.home/dav// "), "https://nas.home/dav");
    }

    #[test]
    fn addresses_of_folders_and_files() {
        let server = Server { url: "https://nas.home/dav".into(), username: String::new(), password: String::new() };
        let path = vec!["Movies".to_string(), "2026 Films".to_string()];
        assert_eq!(server.folder_url(&[]), "https://nas.home/dav/");
        assert_eq!(server.folder_url(&path), "https://nas.home/dav/Movies/2026%20Films/");
        assert_eq!(server.file_url(&path, "Dune Part Three (2026).mkv"), "https://nas.home/dav/Movies/2026%20Films/Dune%20Part%20Three%20(2026).mkv");
        assert_eq!(server.file_url(&[], "电影.mkv"), "https://nas.home/dav/%E7%94%B5%E5%BD%B1.mkv");
        assert_eq!(encode_segment("a/b?c#d%e"), "a%2Fb%3Fc%23d%25e");
    }

    #[test]
    fn decoding_undoes_encoding() {
        for name in ["电影 (2026) #1.mkv", "100% done.txt", "plain", "a+b&c=d"] {
            assert_eq!(decode(&encode_segment(name)), name);
        }
        // A stray "%" or a bad escape is left as it is; bytes that are not UTF-8 are replaced.
        assert_eq!(decode("100%"), "100%");
        assert_eq!(decode("%zz"), "%zz");
        assert_eq!(decode("%FF"), "\u{FFFD}");
    }

    #[test]
    fn path_of_an_address() {
        assert_eq!(url_path("https://nas.home:5006/dav/Media/"), "/dav/Media");
        assert_eq!(url_path("https://nas.home"), "/");
        assert_eq!(url_path("https://nas.home/"), "/");
        assert_eq!(url_path("/dav/%E7%94%B5%E5%BD%B1/"), "/dav/电影");
        assert_eq!(url_path("/dav/x?y=1#z"), "/dav/x");
    }

    #[test]
    fn the_authorization_header() {
        let server = Server { url: "http://nas.home".into(), username: "yumin".into(), password: "pass".into() };
        assert_eq!(server.authorization().as_deref(), Some("Basic eXVtaW46cGFzcw=="));
        let nobody = Server { username: String::new(), ..server };
        assert_eq!(nobody.authorization(), None);
    }

    const LISTING: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>/dav/Movies/</D:href>
    <D:propstat>
      <D:prop><D:resourcetype><D:collection/></D:resourcetype><D:displayname>Movies</D:displayname></D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>/dav/Movies/2026%20Films/</D:href>
    <D:propstat>
      <D:prop><D:resourcetype><D:collection/></D:resourcetype><D:getlastmodified>Wed, 30 Sep 2026 10:00:00 GMT</D:getlastmodified></D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>http://nas.home/dav/Movies/Dune%20Part%20Three%20(2026).mkv</D:href>
    <D:propstat>
      <D:prop><D:getcontentlength>4718592000</D:getcontentlength><D:resourcetype/><D:getlastmodified>Tue, 29 Sep 2026 08:30:00 GMT</D:getlastmodified></D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
    <D:propstat>
      <D:prop><D:displayname/></D:prop>
      <D:status>HTTP/1.1 404 Not Found</D:status>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>/dav/Movies/notes.txt</D:href>
    <D:propstat>
      <D:prop><D:resourcetype/></D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
</D:multistatus>"#;

    #[test]
    fn a_listing_is_the_folders_and_files_but_the_folder() {
        let entries = parse_listing(LISTING, "/dav/Movies").expect("a listing");
        assert_eq!(
            entries,
            vec![
                Entry { name: "2026 Films".into(), is_dir: true, size: 0, modified: Some(1790762400) },
                Entry { name: "Dune Part Three (2026).mkv".into(), is_dir: false, size: 4718592000, modified: Some(1790670600) },
                Entry { name: "notes.txt".into(), is_dir: false, size: 0, modified: None },
            ]
        );
    }

    #[test]
    fn a_listing_in_the_default_namespace() {
        let xml = r#"<multistatus xmlns="DAV:"><response><href>/</href><propstat><prop><resourcetype><collection/></resourcetype></prop><status>HTTP/1.1 200 OK</status></propstat></response>
            <response><href>/film.mkv</href><propstat><prop><getcontentlength>12</getcontentlength></prop><status>HTTP/1.1 200 OK</status></propstat></response></multistatus>"#;
        let entries = parse_listing(xml, "/").expect("a listing");
        assert_eq!(entries, vec![Entry { name: "film.mkv".into(), is_dir: false, size: 12, modified: None }]);
    }

    #[test]
    fn what_is_not_a_listing() {
        assert!(matches!(parse_listing("<html><body>Login</body></html>", "/"), Err(Error::Unreadable(_))));
        assert!(matches!(parse_listing("not xml at all", "/"), Err(Error::Unreadable(_))));
    }
}
