//! The Subsonic API (subsonic.org's REST API 1.16.1, with OpenSubsonic's extensions,
//! opensubsonic.netlify.app): what a music source speaks. Navidrome, gonic, LMS, Ampache,
//! Nextcloud Music, Koel and the rest answer it, in JSON here. Every call is blocking, on the
//! music module's worker thread (src/music/mod.rs); nothing here touches the UI.
//!
//! The password never goes over the wire as it is: each request carries a salt and the MD5 of
//! the password and the salt, the API's own rule (token authentication, 1.13.0). A server that
//! cannot take that (error 41, LDAP accounts) gets the password hex-encoded instead, once asked.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use md5::{Digest as _, Md5};
use ring::rand::{SecureRandom as _, SystemRandom};
use serde::Deserialize;

/// The REST API version the client speaks.
pub const API_VERSION: &str = "1.16.1";
/// How the client names itself to the server (the `c` parameter; Navidrome lists players by it).
pub const CLIENT_NAME: &str = "FinchGram";
/// A JSON answer larger than this is not read: a whole library's artists is well under it.
const LARGEST_ANSWER: u64 = 64 << 20;
/// A cover larger than this is not read.
const LARGEST_COVER: u64 = 32 << 20;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const CALL_TIMEOUT: Duration = Duration::from_secs(60);

/// What went wrong with a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The server answered the API's way: its code (40 wrong password, 50 not allowed, 70 not
    /// found, 0 anything else) and its words.
    Server { code: i32, message: String },
    /// HTTP said no: the status.
    Http(u16),
    /// The server could not be reached, or did not answer in time: why, in a few words.
    Unreachable(String),
    /// What came back is not a Subsonic answer: the start of it.
    Unreadable(String),
}

impl Error {
    /// The account was refused: the password, or the way it was sent.
    pub fn is_password(&self) -> bool {
        matches!(self, Error::Server { code: 40 | 41 | 42 | 44, .. })
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Server { code, message } => write!(f, "the server says: {message} ({code})"),
            Error::Http(status) => write!(f, "HTTP {status}"),
            Error::Unreachable(why) => write!(f, "cannot reach the server: {why}"),
            Error::Unreadable(start) => write!(f, "not a Subsonic answer: {start}"),
        }
    }
}

/// An HTTP client for the API: the Keychain's roots trusted too (a server of the user's own,
/// behind a certificate of their own), each call capped in time, every status an answer to read.
pub fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .user_agent(format!("finchgram/{}", crate::update::CURRENT_VERSION))
        .tls_config(ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build())
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_global(Some(CALL_TIMEOUT))
        .max_redirects(0)
        .http_status_as_error(false)
        .build()
        .into()
}

/// A server, with the account it gave the user.
#[derive(Clone)]
pub struct Client {
    /// The server's root, "https://nas.home:4533" or "https://nas.home/music", without "/rest".
    base: String,
    username: String,
    password: String,
    agent: ureq::Agent,
    /// The server cannot take a token (error 41): the password goes hex-encoded.
    legacy: std::sync::Arc<AtomicBool>,
}

impl Client {
    /// `address` as the user typed it ("https://nas.home", a folder may be part of it), `port`
    /// 0 for the address's own.
    pub fn new(address: &str, port: u16, username: &str, password: &str, agent: ureq::Agent) -> Client {
        Client { base: base_url(address, port), username: username.to_string(), password: password.to_string(), agent, legacy: Default::default() }
    }

    /// What the server says of itself: an answer at all means the account is good.
    pub fn ping(&self) -> Result<ServerInfo, Error> {
        let answer = self.call("ping", &[])?;
        Ok(server_info(&answer))
    }

    /// OpenSubsonic's extensions the server has; none from a server that has not heard of them.
    pub fn extensions(&self) -> Result<Vec<Extension>, Error> {
        match self.call("getOpenSubsonicExtensions", &[]) {
            Ok(answer) => Ok(serde_json::from_value(answer.get("openSubsonicExtensions").cloned().unwrap_or_default()).unwrap_or_default()),
            Err(Error::Server { code: 0 | 70, .. }) | Err(Error::Http(404)) => Ok(Vec::new()),
            Err(err) => Err(err),
        }
    }

    /// A page of albums, `size` at most (500 is the API's ceiling), from `offset`.
    pub fn albums(&self, list: &AlbumList, size: u32, offset: u32) -> Result<Vec<Album>, Error> {
        let mut params = vec![("type", list.kind().to_string()), ("size", size.to_string()), ("offset", offset.to_string())];
        if let AlbumList::ByGenre(genre) = list {
            params.push(("genre", genre.clone()));
        }
        let answer = self.call("getAlbumList2", &params)?;
        Ok(inner_list(&answer, "albumList2", "album"))
    }

    /// An album with its songs.
    pub fn album(&self, id: &str) -> Result<Album, Error> {
        let answer = self.call("getAlbum", &[("id", id.to_string())])?;
        inner(&answer, "album")
    }

    /// Every artist, in the server's order (its index letters, each in order).
    pub fn artists(&self) -> Result<Vec<Artist>, Error> {
        let answer = self.call("getArtists", &[])?;
        let indexes: Vec<ArtistIndex> = serde_json::from_value(answer.get("artists").and_then(|artists| artists.get("index")).cloned().unwrap_or_default()).unwrap_or_default();
        Ok(indexes.into_iter().flat_map(|index| index.artist).collect())
    }

    /// An artist with their albums.
    pub fn artist(&self, id: &str) -> Result<Artist, Error> {
        let answer = self.call("getArtist", &[("id", id.to_string())])?;
        inner(&answer, "artist")
    }

    pub fn genres(&self) -> Result<Vec<Genre>, Error> {
        let answer = self.call("getGenres", &[])?;
        Ok(inner_list(&answer, "genres", "genre"))
    }

    pub fn playlists(&self) -> Result<Vec<Playlist>, Error> {
        let answer = self.call("getPlaylists", &[])?;
        Ok(inner_list(&answer, "playlists", "playlist"))
    }

    /// A playlist with its songs.
    pub fn playlist(&self, id: &str) -> Result<Playlist, Error> {
        let answer = self.call("getPlaylist", &[("id", id.to_string())])?;
        inner(&answer, "playlist")
    }

    /// What the user starred: artists, albums and songs.
    pub fn starred(&self) -> Result<Starred, Error> {
        let answer = self.call("getStarred2", &[])?;
        inner(&answer, "starred2")
    }

    /// Artists, albums and songs whose names hold `query`; an empty query lists everything, a
    /// page at a time (OpenSubsonic's reading of search3, which Navidrome follows).
    pub fn search(&self, query: &str, counts: Counts) -> Result<Found, Error> {
        let params = [
            ("query", query.to_string()),
            ("artistCount", counts.artists.to_string()),
            ("artistOffset", counts.artist_offset.to_string()),
            ("albumCount", counts.albums.to_string()),
            ("albumOffset", counts.album_offset.to_string()),
            ("songCount", counts.songs.to_string()),
            ("songOffset", counts.song_offset.to_string()),
        ];
        let answer = self.call("search3", &params)?;
        inner(&answer, "searchResult3")
    }

    /// Star, or unstar, a song or an album.
    pub fn star(&self, what: &Starrable, on: bool) -> Result<(), Error> {
        let (name, id) = match what {
            Starrable::Song(id) => ("id", id),
            Starrable::Album(id) => ("albumId", id),
        };
        self.call(if on { "star" } else { "unstar" }, &[(name, id.clone())]).map(|_| ())
    }

    /// Tell the server a song plays (`submission` false, its "now playing"), or was listened to
    /// (true: its play count, and Last.fm or ListenBrainz where the server scrobbles).
    pub fn scrobble(&self, id: &str, submission: bool) -> Result<(), Error> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_millis());
        self.call("scrobble", &[("id", id.to_string()), ("time", now.to_string()), ("submission", submission.to_string())]).map(|_| ())
    }

    /// A cover, `size` pixels on its longer side: its bytes and their content type.
    pub fn cover_art(&self, id: &str, size: u32) -> Result<(Vec<u8>, String), Error> {
        let url = self.url("getCoverArt", &[("id", id.to_string()), ("size", size.to_string())]);
        let mut response = self.agent.get(&url).call().map_err(unreachable)?;
        let status = response.status();
        let kind = response.headers().get("content-type").and_then(|value| value.to_str().ok()).unwrap_or("").to_string();
        if !status.is_success() {
            return Err(Error::Http(status.as_u16()));
        }
        if kind.starts_with("application/json") || kind.starts_with("text/xml") {
            // The API's own error, as JSON: no such cover, or the account refused.
            let text = response.body_mut().with_config().limit(LARGEST_ANSWER).read_to_string().map_err(unreachable)?;
            return Err(parse(&text).err().unwrap_or(Error::Unreadable(text.chars().take(80).collect())));
        }
        let bytes = response.body_mut().with_config().limit(LARGEST_COVER).read_to_vec().map_err(unreachable)?;
        Ok((bytes, kind))
    }

    /// Where a song's bytes come from, as the server sends them (transcoded where the server's
    /// settings for this client say so): what the fetcher reads (src/music/fetch.rs).
    pub fn stream_url(&self, id: &str) -> String {
        self.url("stream", &[("id", id.to_string())])
    }

    /// The URL of `method` with the account's parameters and `params`.
    fn url(&self, method: &str, params: &[(&str, String)]) -> String {
        let mut url = format!("{}/rest/{method}?u={}", self.base, encode(&self.username));
        if self.legacy.load(Ordering::Relaxed) {
            url.push_str("&p=enc:");
            url.push_str(&hex(self.password.as_bytes()));
        } else {
            let salt = salt();
            url.push_str("&t=");
            url.push_str(&token(&self.password, &salt));
            url.push_str("&s=");
            url.push_str(&salt);
        }
        url.push_str(&format!("&v={API_VERSION}&c={CLIENT_NAME}&f=json"));
        for (name, value) in params {
            url.push('&');
            url.push_str(name);
            url.push('=');
            url.push_str(&encode(value));
        }
        url
    }

    /// `method` called: the "subsonic-response" object of a good answer. A server that cannot
    /// take a token (41) is asked once more with the password hex-encoded, and so from then on.
    fn call(&self, method: &str, params: &[(&str, String)]) -> Result<serde_json::Value, Error> {
        match self.call_once(method, params) {
            Err(Error::Server { code: 41, .. }) if !self.legacy.load(Ordering::Relaxed) => {
                self.legacy.store(true, Ordering::Relaxed);
                self.call_once(method, params)
            }
            other => other,
        }
    }

    fn call_once(&self, method: &str, params: &[(&str, String)]) -> Result<serde_json::Value, Error> {
        let url = self.url(method, params);
        let mut response = self.agent.get(&url).call().map_err(unreachable)?;
        let status = response.status();
        let text = response.body_mut().with_config().limit(LARGEST_ANSWER).read_to_string().map_err(unreachable)?;
        match parse(&text) {
            Ok(answer) => Ok(answer),
            // The API's own error comes with HTTP 200; a status that is not success without one
            // is HTTP's answer (a proxy, a wrong address).
            Err(Error::Unreadable(start)) if !status.is_success() => Err(Error::Http(status.as_u16())).or(Err(Error::Unreadable(start))),
            Err(err) => Err(err),
        }
    }
}

fn unreachable(err: ureq::Error) -> Error {
    Error::Unreachable(err.to_string())
}

/// The "subsonic-response" object of a good answer, or the error the server put there.
fn parse(text: &str) -> Result<serde_json::Value, Error> {
    let start = || text.trim().chars().take(80).collect::<String>();
    let mut whole: serde_json::Value = serde_json::from_str(text).map_err(|_| Error::Unreadable(start()))?;
    let Some(answer) = whole.get_mut("subsonic-response").map(serde_json::Value::take) else { return Err(Error::Unreadable(start())) };
    if answer.get("status").and_then(serde_json::Value::as_str) == Some("ok") {
        return Ok(answer);
    }
    let code = answer.get("error").and_then(|error| error.get("code")).and_then(serde_json::Value::as_i64).unwrap_or(0) as i32;
    let message = answer.get("error").and_then(|error| error.get("message")).and_then(serde_json::Value::as_str).unwrap_or("failed").to_string();
    Err(Error::Server { code, message })
}

fn server_info(answer: &serde_json::Value) -> ServerInfo {
    let text = |name: &str| answer.get(name).and_then(serde_json::Value::as_str).unwrap_or("").to_string();
    ServerInfo { version: text("version"), kind: text("type"), server_version: text("serverVersion"), open_subsonic: answer.get("openSubsonic").and_then(serde_json::Value::as_bool).unwrap_or(false) }
}

/// `answer[name]` as a `T`.
fn inner<T: serde::de::DeserializeOwned>(answer: &serde_json::Value, name: &str) -> Result<T, Error> {
    let value = answer.get(name).cloned().ok_or_else(|| Error::Unreadable(format!("no {name} in the answer")))?;
    serde_json::from_value(value).map_err(|err| Error::Unreadable(format!("{name}: {err}")))
}

/// `answer[name][item]` as a list; none when the list is missing (an empty list, with most servers).
fn inner_list<T: serde::de::DeserializeOwned>(answer: &serde_json::Value, name: &str, item: &str) -> Vec<T> {
    answer.get(name).and_then(|holder| holder.get(item)).cloned().and_then(|value| serde_json::from_value(value).ok()).unwrap_or_default()
}

/// The server's root from the address as typed and the port: a trailing "/" or "/rest" dropped,
/// the port put after the host unless the address has one of its own.
pub fn base_url(address: &str, port: u16) -> String {
    let mut base = address.trim().trim_end_matches('/').to_string();
    if base.len() >= 5 && base[base.len() - 5..].eq_ignore_ascii_case("/rest") {
        base.truncate(base.len() - 5);
        base = base.trim_end_matches('/').to_string();
    }
    if port == 0 {
        return base;
    }
    let Some(scheme_end) = base.find("://") else { return base };
    let host_start = scheme_end + 3;
    let host_end = base[host_start..].find('/').map_or(base.len(), |slash| host_start + slash);
    let host = &base[host_start..host_end];
    let has_port = if host.starts_with('[') { host.contains("]:") } else { host.contains(':') };
    if has_port {
        return base;
    }
    format!("{}:{port}{}", &base[..host_end], &base[host_end..])
}

/// The host of an address: "nas.home" for "https://nas.home:4533/music".
pub fn host_of(address: &str) -> String {
    let rest = address.trim().split_once("://").map_or(address.trim(), |(_, rest)| rest);
    let host = rest.split('/').next().unwrap_or(rest);
    if host.starts_with('[') {
        host.split("]:").next().map_or(host, |ipv6| ipv6.trim_start_matches('[').trim_end_matches(']')).to_string()
    } else {
        host.split(':').next().unwrap_or(host).to_string()
    }
}

/// Whether the address is one the client can call: http:// or https:// and a host.
pub fn is_address(address: &str) -> bool {
    let address = address.trim();
    let lower = address.to_ascii_lowercase();
    let rest = if let Some(rest) = lower.strip_prefix("https://") {
        rest
    } else if let Some(rest) = lower.strip_prefix("http://") {
        rest
    } else {
        return false;
    };
    let host = rest.split('/').next().unwrap_or("");
    !host.is_empty() && !host.contains(char::is_whitespace)
}

/// The API's token: MD5 of the password followed by the salt, as hex.
pub fn token(password: &str, salt: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(password.as_bytes());
    hasher.update(salt.as_bytes());
    hex(&hasher.finalize())
}

/// Twelve hex digits of randomness: a new salt for every request.
fn salt() -> String {
    let mut bytes = [0u8; 6];
    if SystemRandom::new().fill(&mut bytes).is_err() {
        // No randomness from the system: the time serves; the salt is no secret, only a change.
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_nanos());
        bytes.copy_from_slice(&now.to_le_bytes()[..6]);
    }
    hex(&bytes)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// `value` for a query string: everything but RFC 3986's unreserved characters escaped.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

// ---- what the server says --------------------------------------------------------------------

/// What `ping` tells of the server.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerInfo {
    /// The API version it speaks, "1.16.1".
    pub version: String,
    /// What it is: "navidrome", "gonic", "subsonic", …; "" from a server that does not say.
    pub kind: String,
    /// Its own version, "0.64.2 (10114574)".
    pub server_version: String,
    pub open_subsonic: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Extension {
    pub name: String,
    pub versions: Vec<u32>,
}

/// Which albums `getAlbumList2` lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlbumList {
    Newest,
    ByName,
    ByArtist,
    Frequent,
    Recent,
    Random,
    ByGenre(String),
}

impl AlbumList {
    fn kind(&self) -> &'static str {
        match self {
            AlbumList::Newest => "newest",
            AlbumList::ByName => "alphabeticalByName",
            AlbumList::ByArtist => "alphabeticalByArtist",
            AlbumList::Frequent => "frequent",
            AlbumList::Recent => "recent",
            AlbumList::Random => "random",
            AlbumList::ByGenre(_) => "byGenre",
        }
    }
}

/// How many of each `search3` returns, and from where.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub artists: u32,
    pub artist_offset: u32,
    pub albums: u32,
    pub album_offset: u32,
    pub songs: u32,
    pub song_offset: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Starrable {
    Song(String),
    Album(String),
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Album {
    pub id: String,
    pub name: String,
    pub artist: String,
    pub artist_id: String,
    pub cover_art: String,
    pub song_count: u32,
    /// Seconds.
    pub duration: u64,
    pub year: Option<i32>,
    pub genre: String,
    /// When the user starred it; none when they did not.
    pub starred: Option<String>,
    pub created: String,
    pub play_count: u64,
    /// OpenSubsonic: the artists as one line, "Miles Davis feat. …"; "" from an older server.
    pub display_artist: String,
    /// The songs, from `getAlbum`.
    pub song: Vec<Song>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Song {
    pub id: String,
    pub title: String,
    pub album: String,
    pub artist: String,
    pub track: Option<u32>,
    pub disc_number: Option<u32>,
    pub year: Option<i32>,
    pub cover_art: String,
    /// Bytes of the file as it is on the server.
    pub size: u64,
    pub content_type: String,
    /// "mp3", "flac", …
    pub suffix: String,
    /// Seconds.
    pub duration: u64,
    pub bit_rate: u32,
    pub album_id: String,
    pub artist_id: String,
    pub starred: Option<String>,
    pub path: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Artist {
    pub id: String,
    pub name: String,
    pub album_count: u32,
    pub cover_art: String,
    pub starred: Option<String>,
    /// The albums, from `getArtist`.
    pub album: Vec<Album>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
struct ArtistIndex {
    name: String,
    artist: Vec<Artist>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Genre {
    pub value: String,
    pub song_count: u32,
    pub album_count: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub song_count: u32,
    /// Seconds.
    pub duration: u64,
    pub owner: String,
    pub public: bool,
    pub created: String,
    pub changed: String,
    pub cover_art: String,
    /// The songs, from `getPlaylist`.
    pub entry: Vec<Song>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Starred {
    pub artist: Vec<Artist>,
    pub album: Vec<Album>,
    pub song: Vec<Song>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Found {
    pub artist: Vec<Artist>,
    pub album: Vec<Album>,
    pub song: Vec<Song>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the answers of a Navidrome 0.64.2 (demo.navidrome.org, 2026-10-31) look like, cut to
    /// a few entries each.
    fn answer(name: &str) -> serde_json::Value {
        let text = match name {
            "ping" => include_str!("fixtures/ping.json"),
            "ping_token_auth" => include_str!("fixtures/ping_token_auth.json"),
            "extensions" => include_str!("fixtures/extensions.json"),
            "music_folders" => include_str!("fixtures/music_folders.json"),
            "album_list_newest" => include_str!("fixtures/album_list_newest.json"),
            "album_list_alpha_page2" => include_str!("fixtures/album_list_alpha_page2.json"),
            "album" => include_str!("fixtures/album.json"),
            "song" => include_str!("fixtures/song.json"),
            "artists" => include_str!("fixtures/artists.json"),
            "artist" => include_str!("fixtures/artist.json"),
            "artist_info2" => include_str!("fixtures/artist_info2.json"),
            "genres" => include_str!("fixtures/genres.json"),
            "playlists" => include_str!("fixtures/playlists.json"),
            "playlist" => include_str!("fixtures/playlist.json"),
            "starred2" => include_str!("fixtures/starred2.json"),
            "search3" => include_str!("fixtures/search3.json"),
            "random_songs" => include_str!("fixtures/random_songs.json"),
            "lyrics_by_song_id" => include_str!("fixtures/lyrics_by_song_id.json"),
            _ => panic!("no fixture {name}"),
        };
        parse(text).expect("a good answer")
    }

    #[test]
    fn a_good_answer_is_opened_and_a_failed_one_is_the_servers_error() {
        let info = server_info(&answer("ping"));
        assert_eq!(info, ServerInfo { version: "1.16.1".into(), kind: "navidrome".into(), server_version: "0.64.2 (10114574)".into(), open_subsonic: true });
        assert!(server_info(&answer("ping_token_auth")).open_subsonic);
        let failed = parse(include_str!("fixtures/error_wrong_password.json")).expect_err("failed");
        assert_eq!(failed, Error::Server { code: 40, message: "Wrong username or password".into() });
        assert!(failed.is_password());
        assert_eq!(parse("<html>nope</html>"), Err(Error::Unreadable("<html>nope</html>".into())));
        assert_eq!(parse(r#"{"other":1}"#), Err(Error::Unreadable(r#"{"other":1}"#.into())));
    }

    #[test]
    fn the_extensions_are_listed_with_their_versions() {
        let extensions: Vec<Extension> = serde_json::from_value(answer("extensions").get("openSubsonicExtensions").cloned().unwrap()).expect("extensions");
        assert_eq!(extensions.len(), 7);
        let lyrics = extensions.iter().find(|extension| extension.name == "songLyrics").expect("songLyrics");
        assert_eq!(lyrics.versions, vec![1, 2]);
        assert!(extensions.iter().any(|extension| extension.name == "transcodeOffset"));
    }

    #[test]
    fn albums_come_in_pages_and_an_album_brings_its_songs() {
        let newest: Vec<Album> = inner_list(&answer("album_list_newest"), "albumList2", "album");
        assert_eq!(newest.len(), 6);
        let first = &newest[0];
        assert_eq!((first.id.as_str(), first.name.as_str(), first.song_count, first.artist.as_str()), ("6dGMsiesuYw9lti9yGIGWY", "netBloc Vol. 42: Live, The Universe & Everything", 11, "Various Artists"));
        assert_eq!(first.year, Some(2013));
        assert!(first.cover_art.starts_with("al-"));
        let page: Vec<Album> = inner_list(&answer("album_list_alpha_page2"), "albumList2", "album");
        assert_eq!(page.len(), 3);
        let album: Album = inner(&answer("album"), "album").expect("the album");
        assert_eq!((album.song.len(), album.duration, album.starred.is_some()), (11, 2954, true));
        let song = &album.song[0];
        assert_eq!((song.title.as_str(), song.track, song.duration, song.suffix.as_str(), song.size, song.bit_rate), ("Feijão", Some(1), 218, "mp3", 5365035, 192));
        assert_eq!((song.album_id.as_str(), song.content_type.as_str(), song.starred.is_some()), ("6dGMsiesuYw9lti9yGIGWY", "audio/mpeg", true));
        let one: Song = inner(&answer("song"), "song").expect("the song");
        assert_eq!(one.id, song.id);
        let random: Vec<Song> = inner_list(&answer("random_songs"), "randomSongs", "song");
        assert_eq!(random.len(), 3);
    }

    #[test]
    fn artists_genres_playlists_and_the_starred_are_read() {
        let indexes: Vec<ArtistIndex> = serde_json::from_value(answer("artists").get("artists").unwrap().get("index").cloned().unwrap()).expect("indexes");
        assert_eq!((indexes.len(), indexes[0].name.as_str(), indexes[0].artist[0].name.as_str()), (3, "#", "2 Mello"));
        let artist: Artist = inner(&answer("artist"), "artist").expect("the artist");
        assert_eq!((artist.name.as_str(), artist.album.is_empty()), ("Various Artists", false));
        assert_eq!(artist.album_count as usize, artist.album.len());
        let genres: Vec<Genre> = inner_list(&answer("genres"), "genres", "genre");
        assert!(genres.len() > 5 && genres.iter().all(|genre| !genre.value.is_empty()));
        let playlists: Vec<Playlist> = inner_list(&answer("playlists"), "playlists", "playlist");
        assert_eq!(playlists.len(), 3);
        let playlist: Playlist = inner(&answer("playlist"), "playlist").expect("the playlist");
        assert_eq!(playlist.entry.len(), 4);
        let starred: Starred = inner(&answer("starred2"), "starred2").expect("starred");
        assert_eq!((starred.artist.len(), starred.album.len(), starred.song.len()), (0, 2, 2));
        // An answer without a list is an empty one.
        let none: Vec<Playlist> = inner_list(&answer("music_folders"), "playlists", "playlist");
        assert!(none.is_empty());
        // Lyrics the server does not have: an empty list, not an error.
        assert!(answer("lyrics_by_song_id").get("lyricsList").is_some());
        assert_eq!(answer("artist_info2").get("artistInfo2").map(|info| info.is_object()), Some(true));
    }

    #[test]
    fn a_search_may_find_only_songs() {
        let found: Found = inner(&answer("search3"), "searchResult3").expect("found");
        assert_eq!((found.artist.len(), found.album.len(), found.song.len()), (0, 0, 3));
    }

    #[test]
    fn the_token_is_the_md5_of_the_password_and_the_salt() {
        // The API's own example (subsonic.org/pages/api.jsp).
        assert_eq!(token("sesame", "c19b2d"), "26719a1196d2a940705a59634eb18eab");
        assert_eq!(salt().len(), 12);
        assert_ne!(salt(), salt());
    }

    #[test]
    fn the_base_url_takes_the_port_and_drops_a_trailing_rest() {
        assert_eq!(base_url("https://nas.home/", 4533), "https://nas.home:4533");
        assert_eq!(base_url("https://nas.home:4533/music/rest/", 0), "https://nas.home:4533/music");
        assert_eq!(base_url("https://nas.home/music", 4533), "https://nas.home:4533/music");
        assert_eq!(base_url("http://10.0.0.5", 0), "http://10.0.0.5");
        assert_eq!(base_url("http://[fe80::1]:8080/x", 4533), "http://[fe80::1]:8080/x");
        assert_eq!(base_url("http://[fe80::1]/x", 4533), "http://[fe80::1]:4533/x");
        assert_eq!(host_of("https://nas.home:4533/music"), "nas.home");
        assert_eq!(host_of("http://[fe80::1]:8080/x"), "fe80::1");
        assert!(is_address("https://nas.home") && is_address("HTTP://10.0.0.5:4533/music"));
        assert!(!is_address("nas.home") && !is_address("https://") && !is_address("ftp://x"));
    }

    /// Against a real server: FINCHGRAM_SUBSONIC_URL, _USER and _PASSWORD name it
    /// (demo.navidrome.org with demo/demo answers). Lists, fetches the first song's start through
    /// the fetcher, and reads a cover.
    #[test]
    #[ignore = "needs a server: FINCHGRAM_SUBSONIC_URL=https://demo.navidrome.org FINCHGRAM_SUBSONIC_USER=demo FINCHGRAM_SUBSONIC_PASSWORD=demo cargo test live_subsonic -- --ignored"]
    fn live_subsonic_server_lists_streams_and_shows_covers() {
        let (Ok(url), Ok(user), Ok(password)) = (std::env::var("FINCHGRAM_SUBSONIC_URL"), std::env::var("FINCHGRAM_SUBSONIC_USER"), std::env::var("FINCHGRAM_SUBSONIC_PASSWORD")) else {
            panic!("set FINCHGRAM_SUBSONIC_URL, FINCHGRAM_SUBSONIC_USER and FINCHGRAM_SUBSONIC_PASSWORD");
        };
        let client = Client::new(&url, 0, &user, &password, agent());
        let info = client.ping().expect("ping");
        eprintln!("server: {info:?}, extensions: {:?}", client.extensions().expect("extensions"));
        let albums = client.albums(&AlbumList::Newest, 5, 0).expect("albums");
        assert!(!albums.is_empty());
        let album = client.album(&albums[0].id).expect("the album");
        assert!(!album.song.is_empty());
        let artists = client.artists().expect("artists");
        assert!(!artists.is_empty());
        let word = albums[0].name.split_whitespace().find(|word| word.chars().filter(|c| c.is_alphanumeric()).count() >= 3).unwrap_or("the");
        let found = client.search(word, Counts { songs: 3, albums: 3, artists: 3, ..Counts::default() }).expect("search");
        assert!(!found.song.is_empty() || !found.album.is_empty() || !found.artist.is_empty(), "nothing found for {word:?}");
        // An empty query lists everything, a page at a time (the list of every song).
        let page = client.search("", Counts { songs: 5, ..Counts::default() }).expect("every song");
        assert_eq!(page.song.len(), 5);
        let _ = client.genres().expect("genres");
        let _ = client.playlists().expect("playlists");
        let _ = client.starred().expect("starred");
        if !album.cover_art.is_empty() {
            let (bytes, kind) = client.cover_art(&album.cover_art, 300).expect("the cover");
            assert!(bytes.len() > 100 && kind.starts_with("image/"), "{kind} {}", bytes.len());
        }
        // The first song, fetched through the fetcher as mpv would read it.
        let song = &album.song[0];
        let folder = std::env::temp_dir().join(format!("finchgram-live-subsonic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let fetch = super::super::fetch::Fetch::start(agent(), client.stream_url(&song.id), folder.join("song.part"), folder.join("song.bin"), song.size, |_, _| {});
        let start = std::time::Instant::now();
        while !fetch.is_done() && !fetch.is_failed() && start.elapsed() < Duration::from_secs(120) {
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(fetch.is_done(), "the song came whole (failed: {})", fetch.is_failed());
        let bytes = std::fs::metadata(folder.join("song.bin")).expect("the file").len();
        eprintln!("fetched {} ({} bytes, server said {}) in {:?}", song.title, bytes, song.size, start.elapsed());
        assert!(bytes > 0);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_request_carries_the_account_as_a_token_and_the_parameters_escaped() {
        let client = Client::new("https://nas.home", 4533, "me", "secret", agent());
        let url = client.url("getAlbum", &[("id", "a b/c&d".to_string())]);
        assert!(url.starts_with("https://nas.home:4533/rest/getAlbum?u=me&t="), "{url}");
        assert!(url.contains("&s=") && url.contains("&v=1.16.1&c=FinchGram&f=json&id=a%20b%2Fc%26d"), "{url}");
        assert!(!url.contains("secret"));
        let salt = url.split("&s=").nth(1).and_then(|rest| rest.split('&').next()).expect("the salt");
        assert!(url.contains(&format!("&t={}&", token("secret", salt))));
        // A server that cannot take a token gets the password hex-encoded.
        client.legacy.store(true, Ordering::Relaxed);
        let url = client.url("ping", &[]);
        assert!(url.contains("&p=enc:736563726574&") && !url.contains("&t="), "{url}");
        assert!(client.stream_url("x1").contains("/rest/stream?") && client.stream_url("x1").ends_with("&id=x1"));
    }
}
