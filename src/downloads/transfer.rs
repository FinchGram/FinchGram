//! The transfer engine: one file over several connections at once, as vget does it. A probe asks
//! for the first two bytes with a Range header: a 206 answer gives the file's size and says the
//! server does ranges, so the file is cut into chunks of 8 MB, fetched by a pool of workers (12
//! streams by default: over HTTP/2 they share one connection, over HTTP/1.1 each has its own) and
//! written into a pre-sized `.part` file at their offsets by a writer thread. Which chunks are
//! done is kept in `.part.json`, so a stopped download goes on where it was. A chunk that fails
//! is fetched again from the last byte written, up to ten times with a growing pause, whatever
//! the failure but the disk; a server that answers 200 to a range gets one stream. Done, the
//! `.part` file takes its name.
//!
//! The engine is async inside (http.rs: hyper on its own runtime) and blocking outside: `download`
//! runs the whole transfer on the caller's thread with `Http::block_on`, and reports progress to
//! the caller's closure from there.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::Write as _;
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{AUTHORIZATION, RANGE};
use hyper::{Method, Request, Response, Version};
use serde::{Deserialize, Serialize};
use tokio::sync::{Semaphore, mpsc, oneshot};

use super::http::{Failure, Http};
use super::log;

/// Streams per file unless settings say otherwise: vget's 12. Measured against PikPak's CDN, 24
/// and 32 were no faster once the tail is taken, and every stream is a request more for a
/// server that answers 503 when pressed.
pub const DEFAULT_CONNECTIONS: usize = 12;
/// The most streams settings may ask for.
pub const MOST_CONNECTIONS: usize = 64;
/// A chunk: 8 MB, vget's, small enough that a failed one is cheap to fetch again.
const CHUNK_SIZE: u64 = 8 * 1024 * 1024;
/// Tries per chunk before the file is given up on; the count starts over whenever a try brought
/// some bytes.
const ATTEMPTS: u32 = 10;
const FIRST_BACKOFF: Duration = Duration::from_millis(500);
const LONGEST_BACKOFF: Duration = Duration::from_secs(8);
/// How often progress is reported while the workers run.
const PROGRESS_TICK: Duration = Duration::from_millis(50);
/// A stream that brings nothing for this long is given up on, and the chunk tried again.
const STALL_TIMEOUT: Duration = Duration::from_secs(60);
/// Writes waiting for the writer thread before the streams pause.
const WRITES_IN_FLIGHT: usize = 64;
/// A server that answers 429, 502, 503 or 504 is busy, not broken: PikPak does that while it
/// fetches a file from cold storage, and when it is pressed. Then every stream of the file
/// pauses before its next request, and the file waits for the server this long before it is
/// given up on; such answers are not counted as tries.
const BUSY_PAUSE: Duration = Duration::from_secs(2);
const BUSY_PATIENCE: Duration = Duration::from_secs(5 * 60);
/// Room a volume must keep beyond a file: for the system, and for the files of a folder arriving
/// beside it. A file that cannot fit is refused before it fills the disk, not at its end.
const SPACE_MARGIN: u64 = 256 * 1024 * 1024;
/// A file of unknown size is checked against the room left this often, by bytes written.
const SPACE_CHECK_EVERY: u64 = 256 * 1024 * 1024;
/// open(2)'s O_NOFOLLOW: the path's last part may not be a symbolic link, so that a link put
/// into the download folder cannot make the engine write into another file (`open_own`).
#[cfg(any(target_os = "macos", target_os = "ios"))]
const O_NOFOLLOW: i32 = 0x0100;
#[cfg(target_os = "linux")]
const O_NOFOLLOW: i32 = 0o400000;
#[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "linux")))]
const O_NOFOLLOW: i32 = 0;
/// How many streams a transfer may use: `count` per file, and a budget shared by every file of
/// a folder being downloaded at once, so that several files together use no more than one.
#[derive(Clone)]
pub struct Streams {
    pub count: usize,
    budget: Arc<Semaphore>,
}

impl Streams {
    pub fn new(count: usize) -> Streams {
        let count = count.clamp(1, MOST_CONNECTIONS);
        Streams { count, budget: Arc::new(Semaphore::new(count)) }
    }
}

/// What the server speaks, for the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Protocol {
    #[default]
    Unknown,
    Http1,
    Http2,
}

impl Protocol {
    fn of(version: Version) -> Protocol {
        match version {
            Version::HTTP_2 | Version::HTTP_3 => Protocol::Http2,
            _ => Protocol::Http1,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Protocol::Unknown => "",
            Protocol::Http1 => "HTTP/1.1",
            Protocol::Http2 => "HTTP/2",
        }
    }
}

/// How far a transfer is: reported to the caller every 50 ms while bytes come, and when the
/// server says it is busy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
    pub protocol: Protocol,
    /// The HTTP status the server is answering with while it is busy (429, 503 …), or 0.
    pub waiting: u16,
}

/// Where a file is, and the account to get it with (webdav::Server::authorization).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub url: String,
    pub authorization: Option<String>,
}

impl Source {
    /// The address for the log: without an account in it, or a query (a CDN's signed link).
    fn shown(&self) -> String {
        log::shown(&self.url)
    }

    /// A GET of the file, or of `range` of it.
    fn request(&self, http: &Http, range: Option<&str>) -> Result<Request<Full<Bytes>>, Error> {
        let mut builder = http.request(Method::GET, &self.url);
        if let Some(authorization) = &self.authorization {
            builder = builder.header(AUTHORIZATION, authorization.as_str());
        }
        if let Some(range) = range {
            builder = builder.header(RANGE, range);
        }
        builder.body(Full::default()).map_err(|err| Error::Protocol(format!("bad address: {err}")))
    }
}

/// Why a file did not arrive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The user stopped it.
    Cancelled,
    Unauthorized,
    Forbidden,
    /// Any other HTTP error, such as 404.
    Status(u16),
    /// The connection could not be made, or closed before the end.
    Connection(String),
    /// The file could not be written.
    Disk(String),
    /// The server did not do what it said it would (a range ignored), or the address is bad.
    Protocol(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Cancelled => write!(f, "cancelled"),
            Error::Unauthorized => write!(f, "the server refused the password (401)"),
            Error::Forbidden => write!(f, "the server refused access (403)"),
            Error::Status(code) => write!(f, "the server answered HTTP {code}"),
            Error::Connection(why) => write!(f, "the connection failed: {why}"),
            Error::Disk(why) => write!(f, "the file could not be written: {why}"),
            Error::Protocol(why) => write!(f, "{why}"),
        }
    }
}

impl From<Failure> for Error {
    fn from(failure: Failure) -> Self {
        match failure {
            Failure::Connection(why) => Error::Connection(why),
            Failure::Timeout => Error::Connection("the server did not answer in time".into()),
            Failure::Redirect(why) => Error::Protocol(why),
        }
    }
}

fn disk(err: std::io::Error) -> Error {
    Error::Disk(err.to_string())
}

/// The error an HTTP status is, 401 and 403 told apart.
fn status_error(code: u16) -> Error {
    match code {
        401 => Error::Unauthorized,
        403 => Error::Forbidden,
        code => Error::Status(code),
    }
}

/// What the probe found out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Probe {
    /// The file's size, when the server said.
    pub size: Option<u64>,
    /// The server answers a range with that range: several streams can share the file.
    pub ranges: bool,
    pub protocol: Protocol,
}

/// Which chunks of a `.part` file are there: `.part.json`, next to it.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct State {
    size: u64,
    chunk_size: u64,
    chunks: usize,
    done: Vec<bool>,
    /// The address the chunks came from, and the server's Last-Modified for it ("" when it gave
    /// none): a `.part` of another file of the same size, or of this file changed since, is not
    /// gone on from.
    source: String,
    modified: String,
}

impl State {
    /// Whether the state is this file's, as `source` and `modified` name it.
    fn is_of(&self, source: &str, modified: &str) -> bool {
        self.source == source && self.modified == modified
    }
}

/// What the workers of one file share.
struct Shared {
    /// Streams in flight across every file of the transfer (Streams::budget).
    budget: Arc<Semaphore>,
    /// The file's size, as the first answer said: every later answer must agree.
    total: u64,
    /// The parts still to fetch: every chunk not done, and what the first answer left of its.
    pending: Mutex<VecDeque<Part>>,
    /// Bytes handed to the writer so far, the done chunks of a resumed file counted.
    bytes: AtomicU64,
    /// A worker failed, or the user stopped the file: the others stop.
    abort: AtomicBool,
    failed: Mutex<Option<Error>>,
    /// The parts of chunks on their way, for the tail.
    parts: Mutex<Vec<Part>>,
    /// How many parts of each chunk are still on their way: the chunk is done when the last is.
    parts_left: Mutex<HashMap<usize, usize>>,
    /// The server said it is busy: when it did first, until when the streams hold back, and what
    /// it said.
    busy: Mutex<Busy>,
}

#[derive(Default, Clone, Copy)]
struct Busy {
    since: Option<Instant>,
    until: Option<Instant>,
    code: u16,
}

/// Whether an HTTP status means "busy, ask again in a moment".
fn is_busy(code: u16) -> bool {
    matches!(code, 429 | 502 | 503 | 504)
}

/// How long the streams hold back after a busy answer (next to nothing in the tests).
fn busy_pause() -> Duration {
    if cfg!(test) { Duration::from_millis(10) } else { BUSY_PAUSE }
}

/// How long a server must have been busy before the page says so: a 503 or two in passing is
/// nothing to tell the user about (vget tells of none); a file waiting on a cold server is.
fn busy_shown_after() -> Duration {
    if cfg!(test) { Duration::ZERO } else { Duration::from_secs(5) }
}

impl Shared {
    /// The first failure is the one reported; every failure stops the others.
    fn fail(&self, err: Error) {
        self.abort.store(true, Ordering::Relaxed);
        let mut failed = self.failed.lock().unwrap();
        if failed.is_none() {
            *failed = Some(err);
        }
    }

    /// A busy answer: every stream holds back a moment. Err when the server has been busy for
    /// longer than the file is worth waiting.
    fn note_busy(&self, code: u16) -> Result<(), Error> {
        let mut busy = self.busy.lock().unwrap();
        let now = Instant::now();
        let since = *busy.since.get_or_insert(now);
        busy.until = Some(now + busy_pause());
        busy.code = code;
        if now.duration_since(since) > BUSY_PATIENCE { Err(Error::Status(code)) } else { Ok(()) }
    }

    /// An answer that was not busy: the server is itself again.
    fn note_fine(&self) {
        *self.busy.lock().unwrap() = Busy::default();
    }

    /// The status a busy server keeps answering, once it has for a while; 0 otherwise.
    fn busy_code(&self) -> u16 {
        let busy = self.busy.lock().unwrap();
        match busy.since {
            Some(since) if since.elapsed() >= busy_shown_after() => busy.code,
            _ => 0,
        }
    }

    /// Hold back while the server is busy.
    async fn wait_if_busy(&self) -> Result<(), Error> {
        loop {
            let until = self.busy.lock().unwrap().until;
            match until {
                Some(until) if until > Instant::now() => wait((until - Instant::now()).min(PROGRESS_TICK), &self.abort).await?,
                _ => return Ok(()),
            }
        }
    }
}

/// A stretch of a chunk one stream fetches: from `offset`, which moves as bytes are written, up
/// to `stop`, which an idle stream moves down when it takes the rest.
struct Part {
    chunk: usize,
    offset: Arc<AtomicU64>,
    stop: Arc<AtomicU64>,
}

impl Part {
    fn new(chunk: usize, from: u64, to: u64) -> Part {
        Part { chunk, offset: Arc::new(AtomicU64::new(from)), stop: Arc::new(AtomicU64::new(to)) }
    }

    fn same(&self, other: &Part) -> bool {
        Arc::ptr_eq(&self.offset, &other.offset)
    }
}

impl Clone for Part {
    fn clone(&self) -> Part {
        Part { chunk: self.chunk, offset: self.offset.clone(), stop: self.stop.clone() }
    }
}

/// What the streams hand the writer thread: bytes for an offset, or word that a chunk's last
/// byte has been handed over, answered once it is on disk.
enum Write {
    Data { offset: u64, data: Bytes },
    Done { index: usize, ack: oneshot::Sender<()> },
}

/// Ask the server about the file: its size, and whether it does ranges (the form's test; a
/// download asks with its first chunk instead, `open_once`).
pub fn probe(http: &Http, source: &Source) -> Result<Probe, Error> {
    http.block_on(async {
        let mut response = http.send(source.request(http, Some("bytes=0-1"))?).await?;
        let protocol = Protocol::of(response.version());
        match response.status().as_u16() {
            206 => {
                let total = content_range_total(&response);
                // The two bytes, read to the body's end: a connection with its body read is kept.
                drain(response.body_mut(), 64).await;
                Ok(Probe { size: total, ranges: total.is_some(), protocol })
            }
            // 416: nothing to range over, the file is empty.
            416 => Ok(Probe { size: Some(0), ranges: true, protocol }),
            200 => Ok(Probe { size: content_length(&response), ranges: false, protocol }),
            code => Err(status_error(code)),
        }
    })
}

/// A transfer's first answer: what the probe found out, and the answer itself, whose body is the
/// first chunk (or the whole file, from a server without ranges), still to be read.
struct Head {
    probe: Probe,
    response: Response<Incoming>,
    /// Where the body's bytes start in the file, and where they end (exclusive).
    from: u64,
    to: u64,
}

/// The first request of a transfer asks for a whole chunk, not two bytes: its answer tells the
/// file's size and whether the server does ranges, as a probe would, and brings the first
/// chunk's bytes with it: one round trip less per file, a second on a server that takes a
/// second to answer. A stopped download asks for its first missing chunk instead.
async fn open_once(http: &Http, source: &Source, dest: &Path) -> Result<Head, Error> {
    let (from, to) = first_range(dest, &source.url);
    let started = Instant::now();
    let response = http.send(source.request(http, Some(&format!("bytes={from}-{}", to - 1)))?).await?;
    let protocol = Protocol::of(response.version());
    let status = response.status().as_u16();
    log::line(format!("first {}: {} {status} in {} ms", source.shown(), protocol.name(), started.elapsed().as_millis()));
    match status {
        206 => {
            let range = range_of(&response, from)?;
            let served = (range.last + 1).min(to);
            Ok(Head { probe: Probe { size: range.total, ranges: range.total.is_some(), protocol }, response, from, to: served })
        }
        // 416: nothing at `from`; the file is empty, or smaller than it was when it was stopped.
        416 => {
            let size = content_range_total(&response).unwrap_or(0);
            Ok(Head { probe: Probe { size: Some(size), ranges: true, protocol }, response, from, to: from })
        }
        200 => Ok(Head { probe: Probe { size: content_length(&response), ranges: false, protocol }, response, from: 0, to: 0 }),
        code => Err(status_error(code)),
    }
}

/// What the first request asks for: the first missing chunk of an earlier try of `dest` from the
/// same address, or the smallest chunk from the start.
fn first_range(dest: &Path, source: &str) -> (u64, u64) {
    if let Some(state) = read_state(&with_suffix(dest, ".part.json"))
        && state.source == source
        && state.chunk_size > 0
        && state.done.len() == state.chunks
        && fs::metadata(with_suffix(dest, ".part")).map(|meta| meta.len() == state.size).unwrap_or(false)
        && let Some(index) = state.done.iter().position(|done| !done)
    {
        let start = index as u64 * state.chunk_size;
        return (start, start + chunk_length(index, state.size, state.chunk_size));
    }
    (0, CHUNK_SIZE)
}

/// The first request, tried again like a chunk is: a server that answers 404 or 5xx once is
/// asked again, and one that says it is busy is waited for, `waiting` told the code meanwhile.
async fn open_with_retries(http: &Http, source: &Source, dest: &Path, cancel: &AtomicBool, waiting: &dyn Fn(u16)) -> Result<Head, Error> {
    let mut attempts: u32 = 0;
    let mut busy_since: Option<Instant> = None;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        match open_once(http, source, dest).await {
            Ok(head) => {
                waiting(0);
                return Ok(head);
            }
            Err(Error::Status(code)) if is_busy(code) => {
                let since = *busy_since.get_or_insert_with(Instant::now);
                log::line(format!("first {}: busy, HTTP {code}, for {:.0} s", source.shown(), since.elapsed().as_secs_f64()));
                if since.elapsed() > BUSY_PATIENCE {
                    return Err(Error::Status(code));
                }
                if since.elapsed() >= busy_shown_after() {
                    waiting(code);
                }
                wait(busy_pause(), cancel).await?;
            }
            Err(err) => {
                attempts += 1;
                log::line(format!("first {}: {err}; try {attempts} of {ATTEMPTS}", source.shown()));
                if attempts >= ATTEMPTS {
                    return Err(err);
                }
                wait(backoff(attempts), cancel).await?;
            }
        }
    }
}

/// The whole size out of a Content-Range, "bytes 0-1/12345" or "bytes */12345"; none for "/*".
fn content_range_total(response: &Response<Incoming>) -> Option<u64> {
    let header = response.headers().get("content-range")?.to_str().ok()?;
    header.trim().rsplit('/').next()?.trim().parse().ok()
}

/// What a 206 answer says it carries: "bytes first-last/total", or "/*" for a total not known.
struct ContentRange {
    last: u64,
    total: Option<u64>,
}

/// The Content-Range of a 206 answer to a request for the bytes from `from`, read strictly: it
/// must be there and well formed, start at `from`, end within the total, and the body must be as
/// long as it says (Content-Length). A cache, a proxy or a server that answers with another
/// stretch of the file would otherwise have its bytes written as the asked-for ones, and the
/// chunk marked done.
fn range_of(response: &Response<Incoming>, from: u64) -> Result<ContentRange, Error> {
    let Some(header) = response.headers().get("content-range").and_then(|value| value.to_str().ok()).map(str::trim) else {
        return Err(Error::Protocol("the server answered 206 without a Content-Range".into()));
    };
    let bad = || Error::Protocol(format!("the server's Content-Range cannot be read: {header}"));
    let (unit, rest) = header.split_once(' ').ok_or_else(bad)?;
    if !unit.eq_ignore_ascii_case("bytes") {
        return Err(bad());
    }
    let (range, total) = rest.split_once('/').ok_or_else(bad)?;
    let total = match total.trim() {
        "*" => None,
        digits => Some(digits.parse::<u64>().map_err(|_| bad())?),
    };
    let (first, last) = range.trim().split_once('-').ok_or_else(bad)?;
    let (first, last) = (first.trim().parse::<u64>().map_err(|_| bad())?, last.trim().parse::<u64>().map_err(|_| bad())?);
    if first > last || total.is_some_and(|total| last >= total) {
        return Err(bad());
    }
    if first != from {
        return Err(Error::Protocol(format!("the server answered with the bytes from {first} where those from {from} were asked for")));
    }
    if let Some(length) = content_length(response)
        && length != last - first + 1
    {
        return Err(Error::Protocol(format!("the server's Content-Length ({length}) does not match its Content-Range ({header})")));
    }
    Ok(ContentRange { last, total })
}

/// The server's Last-Modified for the file, as it wrote it; "" when it gave none.
fn last_modified(response: &Response<Incoming>) -> String {
    response.headers().get("last-modified").and_then(|value| value.to_str().ok()).map(|value| value.trim().to_string()).unwrap_or_default()
}

fn content_length(response: &Response<Incoming>) -> Option<u64> {
    response.headers().get("content-length").and_then(|value| value.to_str().ok()).and_then(|value| value.trim().parse().ok())
}

/// Read what is left of `body` and throw it away, up to `most` bytes: a body read to its end
/// leaves its connection in the pool.
async fn drain(body: &mut Incoming, most: usize) {
    let mut left = most;
    while left > 0 {
        match tokio::time::timeout(Duration::from_secs(5), body.frame()).await {
            Ok(Some(Ok(frame))) => left = left.saturating_sub(frame.data_ref().map_or(0, Bytes::len)),
            _ => return,
        }
    }
}

/// Fetch the file at `source` into `dest`, over up to `streams.count` streams, reporting how far
/// it is to `progress` as it goes. A `dest` that is taken gets " (2)" and so on; the path the
/// file got is returned. `cancel` stops it and leaves the `.part` files for a later try.
/// Blocks: call it on a thread of the file's own.
pub fn download(http: &Http, source: &Source, dest: &Path, streams: &Streams, cancel: &AtomicBool, progress: &(dyn Fn(Progress) + Sync)) -> Result<PathBuf, Error> {
    if let Some(folder) = dest.parent() {
        fs::create_dir_all(folder).map_err(disk)?;
    }
    let part = with_suffix(dest, ".part");
    let state_path = with_suffix(dest, ".part.json");
    let started = Instant::now();
    // The first request is one of the transfer's streams too: a folder's files together never
    // have more requests out than the budget.
    let permit = http.block_on(streams.budget.clone().acquire_owned()).map_err(|_| Error::Cancelled)?;
    let head = http.block_on(open_with_retries(http, source, dest, cancel, &|code| progress(Progress { done: 0, total: 0, protocol: Protocol::Unknown, waiting: code })))?;
    let protocol = head.probe.protocol;
    let modified = last_modified(&head.response);
    if let Some(total) = head.probe.size {
        let already = if head.probe.ranges { kept(dest, &source.url) } else { 0 };
        check_space(dest, total.saturating_sub(already))?;
    }
    let report = |done, total, waiting| progress(Progress { done, total, protocol, waiting });
    match head.probe {
        Probe { size: Some(total), ranges: true, .. } if total > 0 => {
            http.block_on(multi(http, source, &modified, &part, &state_path, total, streams, cancel, &report, head, permit))?;
        }
        Probe { size: Some(0), .. } => {
            drop(permit);
            open_own(&part, true)?;
            report(0, 0, 0);
        }
        Probe { size, .. } => {
            let outcome = http.block_on(single(&part, size, cancel, &report, head.response));
            drop(permit);
            outcome?;
        }
    }
    let _ = fs::remove_file(&state_path);
    let target = free_path(dest);
    fs::rename(&part, &target).map_err(disk)?;
    log::line(format!("done {}: {} in {:.1} s", source.shown(), size_words(fs::metadata(&target).map(|meta| meta.len()).unwrap_or(0)), started.elapsed().as_secs_f64()));
    Ok(target)
}

/// "12.3 MB", for the log.
fn size_words(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

/// The chunk a file is cut into: 8 MB whatever its size, as vget has it (a larger chunk for a
/// large file was tried, and the fewer requests were not worth the longer tail).
fn chunk_size(_total: u64) -> u64 {
    CHUNK_SIZE
}

/// Bytes of `dest` that are there from an earlier try from the same address: the done chunks of
/// its `.part` file. (Whether the file changed on the server meanwhile shows at the first answer.)
pub fn kept(dest: &Path, source: &str) -> u64 {
    let Some(state) = read_state(&with_suffix(dest, ".part.json")) else { return 0 };
    if state.source != source || state.chunk_size == 0 || state.done.len() != state.chunks {
        return 0;
    }
    state.done.iter().enumerate().filter(|(_, done)| **done).map(|(index, _)| chunk_length(index, state.size, state.chunk_size)).sum()
}

/// Several streams: the file in chunks, each worker taking the next until none is left and then
/// the tail of another's, the writer thread putting the bytes where they belong. The first
/// answer's body (`head`) is the first chunk, or the start of it, streamed in by a worker of its
/// own while the others fetch the rest.
#[allow(clippy::too_many_arguments)]
async fn multi(
    http: &Http,
    source: &Source,
    modified: &str,
    part: &Path,
    state_path: &Path,
    total: u64,
    streams: &Streams,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, u64, u16),
    head: Head,
    permit: tokio::sync::OwnedSemaphorePermit,
) -> Result<(), Error> {
    let chunk = chunk_size(total);
    let chunks = usize::try_from(total.div_ceil(chunk)).unwrap_or(usize::MAX);
    let file = open_own(part, false)?;
    // An earlier try of the same file, from the same address and unchanged on the server, goes
    // on; anything else starts afresh.
    let earlier = read_state(state_path)
        .filter(|state| state.is_of(&source.url, modified))
        .filter(|state| state.size == total && state.chunk_size == chunk && state.chunks == chunks && state.done.len() == chunks)
        .filter(|_| file.metadata().map(|meta| meta.len() == total).unwrap_or(false));
    let done = match earlier {
        Some(state) => state.done,
        None => {
            file.set_len(total).map_err(disk)?;
            vec![false; chunks]
        }
    };
    let already: u64 = done.iter().enumerate().filter(|(_, done)| **done).map(|(index, _)| chunk_length(index, total, chunk)).sum();
    let mut pending: VecDeque<Part> = (0..chunks)
        .filter(|index| !done[*index])
        .map(|index| {
            let start = index as u64 * chunk;
            Part::new(index, start, start + chunk_length(index, total, chunk))
        })
        .collect();
    let mut parts_left: HashMap<usize, usize> = pending.iter().map(|part| (part.chunk, 1)).collect();
    // The head's bytes, when they are the start of a pending chunk: that chunk's first part,
    // and what is left of the chunk goes first in the queue.
    let mut first: Option<Part> = None;
    if head.to > head.from
        && let Some(at) = pending.iter().position(|part| part.offset.load(Ordering::Relaxed) == head.from)
    {
        let whole = pending.remove(at).expect("the part at `at`");
        let stop = whole.stop.load(Ordering::Relaxed);
        let to = head.to.min(stop);
        if to < stop {
            pending.push_front(Part::new(whole.chunk, to, stop));
            parts_left.insert(whole.chunk, 2);
        }
        first = Some(Part::new(whole.chunk, head.from, to));
    }
    let shared = Arc::new(Shared {
        budget: streams.budget.clone(),
        total,
        pending: Mutex::new(pending),
        bytes: AtomicU64::new(already),
        abort: AtomicBool::new(false),
        failed: Mutex::new(None),
        parts: Mutex::new(Vec::new()),
        parts_left: Mutex::new(parts_left),
        busy: Mutex::new(Busy::default()),
    });
    progress(already, total, 0);
    let (sender, receiver) = mpsc::channel::<Write>(WRITES_IN_FLIGHT);
    let named = Named { source: source.url.clone(), modified: modified.to_string() };
    let writer = Writer::start(file, state_path.to_path_buf(), named, total, chunk, done, receiver, shared.clone());
    let workers = streams.count.min(chunks.max(1));
    log::line(format!("start {}: {}, {} chunks of {}, {} streams, {} kept", source.shown(), size_words(total), chunks, size_words(chunk), workers, size_words(already)));
    let source = Arc::new(source.clone());
    let mut set = tokio::task::JoinSet::new();
    let mut spawned = 0;
    if let Some(first) = first {
        shared.parts.lock().unwrap().push(first.clone());
        set.spawn_on(head_worker(http.clone(), source.clone(), shared.clone(), sender.clone(), first, head.response, permit), http.handle());
        spawned += 1;
    } else {
        drop(permit);
    }
    for _ in spawned..workers {
        set.spawn_on(worker(http.clone(), source.clone(), shared.clone(), sender.clone()), http.handle());
    }
    drop(sender);
    // Progress every tick, and the user's cancel passed on, until the last worker is done.
    let mut ticker = tokio::time::interval(PROGRESS_TICK);
    let mut last = (already, 0u16);
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if cancel.load(Ordering::Relaxed) {
                    shared.abort.store(true, Ordering::Relaxed);
                }
                let now = (shared.bytes.load(Ordering::Relaxed), shared.busy_code());
                if now != last {
                    last = now;
                    progress(now.0, total, now.1);
                }
            }
            next = set.join_next() => match next {
                None => break,
                Some(Ok(())) => {}
                Some(Err(err)) => shared.fail(Error::Connection(format!("a stream's task failed: {err}"))),
            },
        }
    }
    let written = writer.finish();
    progress(shared.bytes.load(Ordering::Relaxed), total, 0);
    if cancel.load(Ordering::Relaxed) {
        return Err(Error::Cancelled);
    }
    if let Some(err) = shared.failed.lock().unwrap().take() {
        return Err(err);
    }
    written
}

/// The thread with the file: it writes what the streams hand it, in order, and marks a chunk
/// done (in `.part.json`) once its last byte is on disk.
struct Writer {
    thread: std::thread::JoinHandle<Result<(), Error>>,
}

/// Whose `.part` it is: the address and the server's Last-Modified, written into `.part.json`.
struct Named {
    source: String,
    modified: String,
}

impl Writer {
    #[allow(clippy::too_many_arguments)]
    fn start(file: File, state_path: PathBuf, named: Named, total: u64, chunk_size: u64, mut done: Vec<bool>, mut receiver: mpsc::Receiver<Write>, shared: Arc<Shared>) -> Writer {
        let thread = std::thread::Builder::new()
            .name("writer".into())
            .spawn(move || {
                while let Some(write) = receiver.blocking_recv() {
                    match write {
                        Write::Data { offset, data } => {
                            if let Err(err) = file.write_all_at(&data, offset) {
                                let err = disk(err);
                                shared.fail(err.clone());
                                // The streams' sends fail from here on, and they stop.
                                return Err(err);
                            }
                        }
                        Write::Done { index, ack } => {
                            done[index] = true;
                            write_state(&state_path, &named, total, chunk_size, &done);
                            let _ = ack.send(());
                        }
                    }
                }
                Ok(())
            })
            .expect("the writer thread");
        Writer { thread }
    }

    /// Wait for the last byte to be written: the streams are done, so the channel closes.
    fn finish(self) -> Result<(), Error> {
        self.thread.join().unwrap_or_else(|_| Err(Error::Disk("the writer stopped".into())))
    }
}

/// One worker: the next chunk, then the tail of another stream's, until there is nothing left,
/// the file failed, or the user stopped it.
async fn worker(http: Http, source: Arc<Source>, shared: Arc<Shared>, writer: mpsc::Sender<Write>) {
    loop {
        if shared.abort.load(Ordering::Relaxed) {
            return;
        }
        // A stream of the transfer's budget, shared with the other files of a folder.
        let Ok(permit) = shared.budget.clone().acquire_owned().await else { return };
        let Some(part) = next_part(&shared) else { return };
        let outcome = fetch_part(&http, &source, &shared, &writer, &part).await;
        drop(permit);
        shared.parts.lock().unwrap().retain(|other| !other.same(&part));
        match outcome {
            Ok(()) => {}
            Err(Error::Cancelled) => return,
            Err(err) => {
                shared.fail(err);
                return;
            }
        }
    }
}

/// The worker with the first answer: it streams that body in, under the permit the first
/// request already holds, then goes on like any other.
async fn head_worker(http: Http, source: Arc<Source>, shared: Arc<Shared>, writer: mpsc::Sender<Write>, part: Part, mut response: Response<Incoming>, permit: tokio::sync::OwnedSemaphorePermit) {
    let from = part.offset.load(Ordering::Relaxed);
    let end = part.stop.load(Ordering::Relaxed) - 1;
    let streamed = stream_body(response.body_mut(), &part, &shared, &writer, from, end, Instant::now(), Duration::ZERO).await;
    drop(response);
    // Done, or not: `fetch_part` finishes the part, from where the head's body stopped if it did.
    let outcome = match streamed {
        Err(err @ Error::Disk(_)) => Err(err),
        Err(Error::Cancelled) => Err(Error::Cancelled),
        Ok(()) | Err(_) => fetch_part(&http, &source, &shared, &writer, &part).await,
    };
    drop(permit);
    shared.parts.lock().unwrap().retain(|other| !other.same(&part));
    match outcome {
        Ok(()) => worker(http, source, shared, writer).await,
        Err(Error::Cancelled) => {}
        Err(err) => shared.fail(err),
    }
}

/// The next part in the queue, if any. (An idle stream taking over the tail of a slow one was
/// tried: a request more each time, which PikPak answered with 503s; vget's streams each take
/// whole chunks and nothing else, and so do these.)
fn next_part(shared: &Shared) -> Option<Part> {
    let part = shared.pending.lock().unwrap().pop_front()?;
    shared.parts.lock().unwrap().push(part.clone());
    Some(part)
}

/// `part`, tried again from the last byte handed over when the stream fails; the chunk is marked
/// done once its last part is.
async fn fetch_part(http: &Http, source: &Source, shared: &Shared, writer: &mpsc::Sender<Write>, part: &Part) -> Result<(), Error> {
    let mut attempts: u32 = 0;
    loop {
        if shared.abort.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let from = part.offset.load(Ordering::Relaxed);
        if from >= part.stop.load(Ordering::Relaxed) {
            break;
        }
        shared.wait_if_busy().await?;
        match fetch_range(http, source, part, shared, writer).await {
            Ok(()) => break,
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            // Busy: every stream holds back a moment, and this one asks again, as often as the
            // file is worth waiting for.
            Err(Error::Status(code)) if is_busy(code) => {
                log::line(format!("chunk {} of {}: busy, HTTP {code}", part.chunk, source.shown()));
                shared.note_busy(code)?;
            }
            // The disk is the one thing trying again will not help. Everything else may pass next
            // time, as vget has it: a connection that drops, a 429 or a 5xx from a busy server, a
            // 404 from a CDN node that does not have the file yet (PikPak does that), a 200 where a
            // 206 was due.
            Err(err @ Error::Disk(_)) => return Err(err),
            Err(err) => {
                attempts = if part.offset.load(Ordering::Relaxed) > from { 1 } else { attempts + 1 };
                log::line(format!("chunk {} of {}: {err}; try {attempts} of {ATTEMPTS}", part.chunk, source.shown()));
                if attempts >= ATTEMPTS {
                    return Err(err);
                }
                wait(backoff(attempts), &shared.abort).await?;
            }
        }
    }
    // The chunk is done once the last byte of its last part is on disk, not when handed over.
    let last = {
        let mut left = shared.parts_left.lock().unwrap();
        let count = left.entry(part.chunk).or_insert(1);
        *count -= 1;
        let last = *count == 0;
        if last {
            left.remove(&part.chunk);
        }
        last
    };
    if last {
        let (ack, written_down) = oneshot::channel();
        writer.send(Write::Done { index: part.chunk, ack }).await.map_err(|_| Error::Cancelled)?;
        written_down.await.map_err(|_| Error::Cancelled)?;
    }
    Ok(())
}

/// A request for the bytes of `part` from its offset to its stop, and its body streamed in.
async fn fetch_range(http: &Http, source: &Source, part: &Part, shared: &Shared, writer: &mpsc::Sender<Write>) -> Result<(), Error> {
    let from = part.offset.load(Ordering::Relaxed);
    let end = part.stop.load(Ordering::Relaxed) - 1;
    let started = Instant::now();
    let mut response = http.send(source.request(http, Some(&format!("bytes={from}-{end}")))?).await?;
    let answered = started.elapsed();
    match response.status().as_u16() {
        206 => {
            let range = range_of(&response, from)?;
            if let Some(total) = range.total
                && total != shared.total
            {
                return Err(Error::Protocol(format!("the file's size on the server changed: {total} bytes where it was {}", shared.total)));
            }
            shared.note_fine();
        }
        200 => return Err(Error::Protocol("the server ignored the range it said it would answer".into())),
        code => return Err(status_error(code)),
    }
    stream_body(response.body_mut(), part, shared, writer, from, end, started, answered).await
}

/// The bytes of a body that starts at `from` and runs to `end`, handed to the writer for where
/// they belong, up to the part's stop (less than `end` when a server sends more than the part);
/// the offset moves as they go.
#[allow(clippy::too_many_arguments)]
async fn stream_body(body: &mut Incoming, part: &Part, shared: &Shared, writer: &mpsc::Sender<Write>, from: u64, end: u64, started: Instant, answered: Duration) -> Result<(), Error> {
    let mut offset = from;
    loop {
        let stop = part.stop.load(Ordering::Relaxed);
        let remaining = stop.saturating_sub(offset);
        if remaining == 0 {
            if stop > end {
                // To the body's end, so that the connection (HTTP/1.1) or the stream is finished
                // properly and the connection stays in the pool.
                drain(body, 64).await;
            }
            let seconds = started.elapsed().as_secs_f64();
            log::line(format!(
                "chunk {}: answered in {} ms, {} in {seconds:.2} s ({:.1} MB/s)",
                part.chunk,
                answered.as_millis(),
                size_words(offset - from),
                (offset - from) as f64 / (1024.0 * 1024.0) / seconds.max(0.001)
            ));
            return Ok(());
        }
        let frame = match tokio::time::timeout(STALL_TIMEOUT, body.frame()).await {
            Err(_) => return Err(Error::Connection("the stream brought nothing for a minute".into())),
            Ok(None) => return Err(Error::Connection("the connection closed before the end of the range".into())),
            Ok(Some(Err(err))) => return Err(Error::Connection(err.to_string())),
            Ok(Some(Ok(frame))) => frame,
        };
        let Ok(mut data) = frame.into_data() else { continue };
        if data.is_empty() {
            continue;
        }
        // Never past the part's end: a server that sends more is cut short, and so is a part
        // whose rest another stream took.
        if data.len() as u64 > remaining {
            data.truncate(remaining as usize);
        }
        let count = data.len() as u64;
        writer.send(Write::Data { offset, data }).await.map_err(|_| Error::Cancelled)?;
        offset += count;
        part.offset.store(offset, Ordering::Relaxed);
        shared.bytes.fetch_add(count, Ordering::Relaxed);
        if shared.abort.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
    }
}

/// How long to wait before try number `attempts + 1`: half a second, doubling up to eight (the
/// tests, which only count the tries, wait next to nothing).
fn backoff(attempts: u32) -> Duration {
    if cfg!(test) {
        return Duration::from_millis(10);
    }
    let mut pause = FIRST_BACKOFF;
    for _ in 1..attempts {
        pause = (pause * 2).min(LONGEST_BACKOFF);
    }
    pause
}

/// Sleep `pause`, unless the file is stopped meanwhile.
async fn wait(pause: Duration, stop: &AtomicBool) -> Result<(), Error> {
    let until = tokio::time::Instant::now() + pause;
    while tokio::time::Instant::now() < until {
        if stop.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        tokio::time::sleep(PROGRESS_TICK.min(until - tokio::time::Instant::now())).await;
    }
    Ok(())
}

/// One stream, from the start: the answer of a server that does not do ranges is the whole
/// file, and here it is streamed in. Nothing to go on from when it fails.
async fn single(part: &Path, size: Option<u64>, cancel: &AtomicBool, progress: &dyn Fn(u64, u64, u16), mut response: Response<Incoming>) -> Result<(), Error> {
    if !response.status().is_success() {
        return Err(status_error(response.status().as_u16()));
    }
    let total = content_length(&response).or(size).unwrap_or(0);
    let mut file = open_own(part, true)?;
    let body = response.body_mut();
    let mut done: u64 = 0;
    let mut checked: u64 = 0;
    let mut reported = tokio::time::Instant::now();
    progress(0, total, 0);
    loop {
        let frame = match tokio::time::timeout(STALL_TIMEOUT, body.frame()).await {
            Err(_) => return Err(Error::Connection("the stream brought nothing for a minute".into())),
            Ok(None) => break,
            Ok(Some(Err(err))) => return Err(Error::Connection(err.to_string())),
            Ok(Some(Ok(frame))) => frame,
        };
        let Ok(data) = frame.into_data() else { continue };
        file.write_all(&data).map_err(disk)?;
        done += data.len() as u64;
        // A server that never ends its body must not fill the disk.
        if done - checked >= SPACE_CHECK_EVERY {
            check_space(part, 0)?;
            checked = done;
        }
        if reported.elapsed() >= PROGRESS_TICK {
            progress(done, total.max(done), 0);
            reported = tokio::time::Instant::now();
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
    }
    progress(done, total.max(done), 0);
    Ok(())
}

/// How long chunk `index` of a file of `total` bytes is.
fn chunk_length(index: usize, total: u64, chunk_size: u64) -> u64 {
    let start = index as u64 * chunk_size;
    total.saturating_sub(start).min(chunk_size)
}

fn read_state(path: &Path) -> Option<State> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

fn write_state(path: &Path, named: &Named, total: u64, chunk_size: u64, done: &[bool]) {
    let state = State { size: total, chunk_size, chunks: done.len(), done: done.to_vec(), source: named.source.clone(), modified: named.modified.clone() };
    match serde_json::to_string(&state) {
        Ok(text) => {
            if let Err(err) = open_own(path, true).and_then(|mut file| file.write_all(text.as_bytes()).map_err(disk)) {
                eprintln!("downloads: cannot write {}: {err}", path.display());
            }
        }
        Err(err) => eprintln!("downloads: cannot describe {}: {err}", path.display()),
    }
}

/// Err when the volume `dest` is on does not have `needed` bytes free, and the margin beyond.
pub fn check_space(dest: &Path, needed: u64) -> Result<(), Error> {
    let folder = dest.parent().filter(|folder| !folder.as_os_str().is_empty()).unwrap_or(dest);
    let Some(free) = crate::platform::free_space(folder) else { return Ok(()) };
    if free < needed.saturating_add(SPACE_MARGIN) {
        return Err(Error::Disk(format!("not enough room on the disk: {} free, the file needs {}", size_words(free), size_words(needed))));
    }
    Ok(())
}

/// Open, or make, one of the transfer's own files (`.part`, `.part.json`), `truncate`d or not:
/// never through a symbolic link, and a regular file once open. A link put into the download
/// folder under the file's name would otherwise have the engine write into whatever it points at.
fn open_own(path: &Path, truncate: bool) -> Result<File, Error> {
    let file = OpenOptions::new().read(true).write(true).create(true).truncate(truncate).custom_flags(O_NOFOLLOW).open(path).map_err(disk)?;
    if !file.metadata().map_err(disk)?.file_type().is_file() {
        return Err(Error::Disk(format!("{} is not a regular file", path.display())));
    }
    Ok(file)
}

/// The partial files of `dest` go: its `.part` and `.part.json`; or, when `dest` is a folder (a
/// folder download stopped half way), every one under it.
pub fn remove_partials(dest: &Path) {
    if dest.is_dir() {
        let Ok(entries) = fs::read_dir(dest) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                remove_partials(&path);
            } else if path.to_string_lossy().ends_with(".part") || path.to_string_lossy().ends_with(".part.json") {
                let _ = fs::remove_file(&path);
            }
        }
        return;
    }
    let _ = fs::remove_file(with_suffix(dest, ".part"));
    let _ = fs::remove_file(with_suffix(dest, ".part.json"));
}

/// `path` with `suffix` after its name: "film.mkv" → "film.mkv.part".
pub fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

/// `dest` itself, or "name (2).ext", "name (3).ext" … when that is taken.
pub fn free_path(dest: &Path) -> PathBuf {
    if !dest.exists() {
        return dest.to_path_buf();
    }
    let folder = dest.parent().map(Path::to_path_buf).unwrap_or_default();
    let name = dest.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem.to_string(), format!(".{extension}")),
        _ => (name.clone(), String::new()),
    };
    (2..)
        .map(|n| folder.join(format!("{stem} ({n}){extension}")))
        .find(|candidate| !candidate.exists())
        .unwrap_or_else(|| folder.join(name))
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::AtomicUsize;

    use super::*;
    use crate::downloads::http;

    /// A server of one file on the loopback: HTTP/1.1 with keep-alive, ranges when told to, a
    /// count of the connections and of the requests with their Range headers, 429 for the next
    /// `failures` chunk requests, 404 for the next `missing` ones, and 503 for the next
    /// `probe_failures` first requests.
    struct FileServer {
        url: String,
        connections: Arc<AtomicUsize>,
        requests: Arc<Mutex<Vec<String>>>,
        failures: Arc<AtomicUsize>,
        missing: Arc<AtomicUsize>,
        probe_failures: Arc<AtomicUsize>,
    }

    fn serve(content: Vec<u8>, ranges: bool, authorization: Option<&str>) -> FileServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
        let port = listener.local_addr().expect("the port").port();
        let connections = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let failures = Arc::new(AtomicUsize::new(0));
        let missing = Arc::new(AtomicUsize::new(0));
        let probe_failures = Arc::new(AtomicUsize::new(0));
        let content = Arc::new(content);
        let authorization = authorization.map(str::to_string);
        let counters = Arc::new(Counters { failures: failures.clone(), missing: missing.clone(), probe_failures: probe_failures.clone() });
        let (opened, seen) = (connections.clone(), requests.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                opened.fetch_add(1, Ordering::SeqCst);
                let (content, seen, counters, authorization) = (content.clone(), seen.clone(), counters.clone(), authorization.clone());
                std::thread::spawn(move || answer(stream, &content, ranges, authorization.as_deref(), &seen, &counters));
            }
        });
        FileServer { url: format!("http://127.0.0.1:{port}/film.mkv"), connections, requests, failures, missing, probe_failures }
    }

    /// How many of the next requests the server turns away, and how.
    struct Counters {
        failures: Arc<AtomicUsize>,
        missing: Arc<AtomicUsize>,
        probe_failures: Arc<AtomicUsize>,
    }

    /// Takes one from `counter` when there is one to take.
    fn take(counter: &AtomicUsize) -> bool {
        counter.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| left.checked_sub(1)).is_ok()
    }

    fn answer(stream: TcpStream, content: &[u8], ranges: bool, authorization: Option<&str>, requests: &Mutex<Vec<String>>, counters: &Counters) {
        let mut reader = BufReader::new(stream.try_clone().expect("a reader"));
        let mut stream = stream;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                return;
            }
            let (mut range, mut given) = (None, None);
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).unwrap_or(0) == 0 {
                    return;
                }
                let header = header.trim_end();
                if header.is_empty() {
                    break;
                }
                if let Some((name, value)) = header.split_once(':') {
                    match name.to_ascii_lowercase().as_str() {
                        "range" => range = Some(value.trim().to_string()),
                        "authorization" => given = Some(value.trim().to_string()),
                        _ => {}
                    }
                }
            }
            requests.lock().unwrap().push(range.clone().unwrap_or_default());
            // The first request of a download asks from byte 0 (a two-byte probe, or the first
            // chunk); the rest are "chunks" here.
            let probe = range.as_deref().is_some_and(|range| range.starts_with("bytes=0-"));
            let chunk = !probe && range.is_some();
            let (status, extra, body): (&str, String, &[u8]) = if authorization.is_some() && given.as_deref() != authorization {
                ("401 Unauthorized", String::new(), b"")
            } else if chunk && take(&counters.failures) {
                ("429 Too Many Requests", String::new(), b"")
            } else if chunk && take(&counters.missing) {
                ("404 Not Found", String::new(), b"")
            } else if probe && take(&counters.probe_failures) {
                ("503 Service Unavailable", String::new(), b"")
            } else if let Some((start, end)) = range.as_deref().filter(|_| ranges).and_then(parse_range) {
                if start >= content.len() {
                    ("416 Range Not Satisfiable", format!("Content-Range: bytes */{}\r\n", content.len()), b"")
                } else {
                    let end = end.min(content.len() - 1);
                    ("206 Partial Content", format!("Content-Range: bytes {start}-{end}/{}\r\nAccept-Ranges: bytes\r\n", content.len()), &content[start..=end])
                }
            } else {
                ("200 OK", String::new(), content)
            };
            let head = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\n{extra}\r\n", body.len());
            if stream.write_all(head.as_bytes()).and_then(|_| stream.write_all(body)).is_err() {
                return;
            }
        }
    }

    fn parse_range(range: &str) -> Option<(usize, usize)> {
        let (start, end) = range.strip_prefix("bytes=")?.split_once('-')?;
        Some((start.parse().ok()?, if end.is_empty() { usize::MAX } else { end.parse().ok()? }))
    }

    /// A folder of the test's own under the system's temporary folder, removed when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Scratch {
            let folder = std::env::temp_dir().join(format!("finchgram-transfer-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&folder);
            fs::create_dir_all(&folder).expect("a scratch folder");
            Scratch(folder)
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn pattern(length: usize) -> Vec<u8> {
        (0..length).map(|i| (i % 251) as u8 ^ (i / 251 % 7) as u8).collect()
    }

    fn source(server: &FileServer) -> Source {
        Source { url: server.url.clone(), authorization: None }
    }

    fn fetch(server: &FileServer, dest: &Path, connections: usize) -> Result<PathBuf, Error> {
        download(http::client(), &source(server), dest, &Streams::new(connections), &AtomicBool::new(false), &|_| {})
    }

    #[test]
    fn the_probe_tells_the_size_and_whether_ranges_work() {
        let http = http::client();
        let with = serve(pattern(1000), true, None);
        assert_eq!(probe(http, &source(&with)).unwrap(), Probe { size: Some(1000), ranges: true, protocol: Protocol::Http1 });
        let without = serve(pattern(1000), false, None);
        assert_eq!(probe(http, &source(&without)).unwrap(), Probe { size: Some(1000), ranges: false, protocol: Protocol::Http1 });
        let empty = serve(Vec::new(), true, None);
        assert_eq!(probe(http, &source(&empty)).unwrap(), Probe { size: Some(0), ranges: true, protocol: Protocol::Http1 });
        let with_range = |value: &str| Response::builder().header("content-range", value).body(Full::<Bytes>::default()).unwrap();
        let _ = with_range;
    }

    #[test]
    fn a_file_arrives_in_chunks_over_several_connections() {
        let content = pattern(2 * CHUNK_SIZE as usize + 12345);
        let server = serve(content.clone(), true, None);
        let scratch = Scratch::new("chunks");
        let dest = scratch.path("film.mkv");
        let seen = Mutex::new(Vec::new());
        let got = download(http::client(), &source(&server), &dest, &Streams::new(4), &AtomicBool::new(false), &|progress| seen.lock().unwrap().push((progress.done, progress.total))).expect("the file");
        assert_eq!(got, dest);
        assert_eq!(fs::read(&dest).unwrap(), content);
        assert!(!with_suffix(&dest, ".part").exists());
        assert!(!with_suffix(&dest, ".part.json").exists());
        let seen = seen.into_inner().unwrap();
        assert_eq!(seen.last(), Some(&(content.len() as u64, content.len() as u64)));
        assert!(seen.windows(2).all(|pair| pair[0].0 <= pair[1].0), "progress never goes back: {seen:?}");
        let requests = server.requests.lock().unwrap().clone();
        // The first request is the first chunk itself, not a two-byte probe; then the other two.
        assert_eq!(requests[0], format!("bytes=0-{}", CHUNK_SIZE - 1));
        assert_eq!(requests.len(), 3, "{requests:?}");
        assert!(covers(&requests, content.len()), "every byte asked for: {requests:?}");
    }

    /// Whether `ranges` ("bytes=a-b"), together, ask for every byte of a file of `len` bytes.
    fn covers(ranges: &[String], len: usize) -> bool {
        let mut spans: Vec<(usize, usize)> = ranges.iter().filter_map(|range| parse_range(range)).collect();
        spans.sort();
        let mut reached = 0;
        for (start, end) in spans {
            if start > reached {
                return false;
            }
            reached = reached.max(end + 1);
        }
        reached >= len
    }

    #[test]
    fn connections_are_kept_between_chunks() {
        // Twelve chunks over two workers take a handful of HTTP/1.1 connections, each serving
        // chunk after chunk; without the body read to its end, every chunk would open one of its
        // own, thirteen in all.
        let content = pattern(11 * CHUNK_SIZE as usize + 7);
        let server = serve(content.clone(), true, None);
        let scratch = Scratch::new("keepalive");
        let dest = scratch.path("film.mkv");
        fetch(&server, &dest, 2).expect("the file");
        assert_eq!(fs::read(&dest).unwrap(), content);
        // Twelve requests for twelve chunks, over far fewer connections than that.
        assert_eq!(server.requests.lock().unwrap().len(), 12);
        let connections = server.connections.load(Ordering::SeqCst);
        assert!(connections <= 4, "{connections} connections for 12 chunks over 2");
    }

    #[test]
    fn a_stopped_download_goes_on_where_it_was() {
        let content = pattern(2 * CHUNK_SIZE as usize + 999);
        let server = serve(content.clone(), true, None);
        let scratch = Scratch::new("resume");
        let dest = scratch.path("film.mkv");
        // An earlier try that got the first chunk.
        let part = with_suffix(&dest, ".part");
        let mut earlier = vec![0; content.len()];
        earlier[..CHUNK_SIZE as usize].copy_from_slice(&content[..CHUNK_SIZE as usize]);
        fs::write(&part, &earlier).unwrap();
        let named = Named { source: source(&server).url, modified: String::new() };
        write_state(&with_suffix(&dest, ".part.json"), &named, content.len() as u64, CHUNK_SIZE, &[true, false, false]);
        assert_eq!(kept(&dest, &named.source), CHUNK_SIZE);
        let got = fetch(&server, &dest, 2).expect("the file");
        assert_eq!(got, dest);
        assert_eq!(fs::read(&dest).unwrap(), content);
        let requests = server.requests.lock().unwrap().clone();
        let lowest = requests.iter().filter_map(|range| parse_range(range)).map(|(start, _)| start).min();
        assert_eq!(lowest, Some(CHUNK_SIZE as usize), "the first chunk is not fetched again: {requests:?}");
        // The first request is the first missing chunk; then the last one.
        assert_eq!(requests.len(), 2);
        assert_eq!(kept(&dest, &named.source), 0);
    }

    #[test]
    fn a_part_of_another_source_or_of_a_changed_file_is_not_gone_on_from() {
        // The same size as the file, but from another address: nothing of it is kept, and the file
        // comes whole from this server.
        let content = pattern(2 * CHUNK_SIZE as usize + 999);
        let server = serve(content.clone(), true, None);
        let scratch = Scratch::new("other-source");
        let dest = scratch.path("film.mkv");
        fs::write(with_suffix(&dest, ".part"), vec![b'A'; content.len()]).unwrap();
        let elsewhere = Named { source: "http://elsewhere.example/film.mkv".into(), modified: String::new() };
        write_state(&with_suffix(&dest, ".part.json"), &elsewhere, content.len() as u64, CHUNK_SIZE, &[true, false, false]);
        assert_eq!(kept(&dest, &source(&server).url), 0);
        fetch(&server, &dest, 2).expect("the file");
        assert_eq!(fs::read(&dest).unwrap(), content);
        let requests = server.requests.lock().unwrap().clone();
        assert_eq!(requests.iter().filter_map(|range| parse_range(range)).map(|(start, _)| start).min(), Some(0), "{requests:?}");
    }

    #[test]
    fn one_stream_when_the_server_does_no_ranges() {
        let content = pattern(100_000);
        let server = serve(content.clone(), false, None);
        let scratch = Scratch::new("single");
        let dest = scratch.path("film.mkv");
        fetch(&server, &dest, 4).expect("the file");
        assert_eq!(fs::read(&dest).unwrap(), content);
        // The first request's answer, a 200 with the whole file, is the download: one request.
        assert_eq!(*server.requests.lock().unwrap(), vec![format!("bytes=0-{}", CHUNK_SIZE - 1)]);
    }

    #[test]
    fn a_busy_server_is_waited_for() {
        // 429 is "busy", waited for as long as it takes, not counted as a try: fifteen of them
        // and the file still comes.
        let content = pattern(CHUNK_SIZE as usize + 100_000);
        let server = serve(content.clone(), true, None);
        server.failures.store(15, Ordering::SeqCst);
        let scratch = Scratch::new("busy");
        let dest = scratch.path("film.mkv");
        fetch(&server, &dest, 1).expect("the file");
        assert_eq!(fs::read(&dest).unwrap(), content);
        // The first chunk, then the second: fifteen tries answered 429, the one that worked.
        assert_eq!(server.requests.lock().unwrap().len(), 17);
    }

    #[test]
    fn a_404_from_a_node_without_the_file_yet_passes() {
        // PikPak's CDN answers 404 for a moment to a range of a file it is still fetching.
        let content = pattern(CHUNK_SIZE as usize + 100_000);
        let server = serve(content.clone(), true, None);
        server.missing.store(1, Ordering::SeqCst);
        let scratch = Scratch::new("missing");
        let dest = scratch.path("film.mkv");
        fetch(&server, &dest, 1).expect("the file");
        assert_eq!(fs::read(&dest).unwrap(), content);
        // The first chunk, the second's 404, the one that worked.
        assert_eq!(server.requests.lock().unwrap().len(), 3);
    }

    #[test]
    fn the_first_request_waits_for_a_busy_server_too() {
        let content = pattern(100_000);
        let server = serve(content.clone(), true, None);
        server.probe_failures.store(12, Ordering::SeqCst);
        let scratch = Scratch::new("probe-retry");
        let dest = scratch.path("film.mkv");
        let waited = Mutex::new(Vec::new());
        download(http::client(), &source(&server), &dest, &Streams::new(4), &AtomicBool::new(false), &|progress| waited.lock().unwrap().push(progress.waiting)).expect("the file");
        assert_eq!(fs::read(&dest).unwrap(), content);
        // Twelve answers of 503, then the one that worked, which was the whole file; the page
        // was told the 503 meanwhile, and that it was over.
        assert_eq!(server.requests.lock().unwrap().len(), 13);
        let waited = waited.into_inner().unwrap();
        assert!(waited.contains(&503) && waited.last() == Some(&0), "{waited:?}");
    }

    #[test]
    fn a_refused_password_and_a_closed_port_are_told_apart() {
        let server = serve(pattern(10), true, Some("Basic abc"));
        let scratch = Scratch::new("refused");
        assert_eq!(fetch(&server, &scratch.path("film.mkv"), 2), Err(Error::Unauthorized));
        let http = http::client();
        let right = Source { url: server.url.clone(), authorization: Some("Basic abc".into()) };
        assert_eq!(download(http, &right, &scratch.path("film.mkv"), &Streams::new(2), &AtomicBool::new(false), &|_| {}), Ok(scratch.path("film.mkv")));
        let gone = Source { url: "http://127.0.0.1:1/film.mkv".into(), authorization: None };
        assert!(matches!(download(http, &gone, &scratch.path("gone.mkv"), &Streams::new(2), &AtomicBool::new(false), &|_| {}), Err(Error::Connection(_))));
    }

    #[test]
    fn cancelled_before_the_first_byte_fetches_nothing() {
        let content = pattern(2 * CHUNK_SIZE as usize);
        let server = serve(content, true, None);
        let scratch = Scratch::new("cancel");
        let dest = scratch.path("film.mkv");
        let cancel = AtomicBool::new(true);
        assert_eq!(download(http::client(), &source(&server), &dest, &Streams::new(2), &cancel, &|_| {}), Err(Error::Cancelled));
        assert!(!dest.exists());
        assert_eq!(kept(&dest, &source(&server).url), 0);
        assert!(server.requests.lock().unwrap().is_empty());
    }

    #[test]
    fn twelve_streams_share_one_http2_connection() {
        // A hyper HTTP/2 server on the loopback (cleartext, by prior knowledge), serving ranges
        // of a file: the twelve chunks over four workers all travel over one TCP connection.
        use hyper::service::service_fn;
        use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
        let content = Arc::new(pattern(11 * CHUNK_SIZE as usize + 7));
        let http = Http::build(true);
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
        listener.set_nonblocking(true).expect("non-blocking");
        let port = listener.local_addr().expect("the port").port();
        let connections = Arc::new(AtomicUsize::new(0));
        let served = content.clone();
        let opened = connections.clone();
        http.handle().spawn(async move {
            let listener = tokio::net::TcpListener::from_std(listener).expect("a tokio listener");
            loop {
                let Ok((stream, _)) = listener.accept().await else { return };
                opened.fetch_add(1, Ordering::SeqCst);
                let served = served.clone();
                tokio::spawn(async move {
                    let service = service_fn(move |request: Request<Incoming>| {
                        let served = served.clone();
                        async move { Ok::<_, std::convert::Infallible>(range_of(&served, &request)) }
                    });
                    let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new()).timer(TokioTimer::new()).serve_connection(TokioIo::new(stream), service).await;
                });
            }
        });
        let source = Source { url: format!("http://127.0.0.1:{port}/film.mkv"), authorization: None };
        let scratch = Scratch::new("h2");
        let dest = scratch.path("film.mkv");
        let seen = Mutex::new(Protocol::Unknown);
        download(&http, &source, &dest, &Streams::new(4), &AtomicBool::new(false), &|progress| *seen.lock().unwrap() = progress.protocol).expect("the file");
        assert_eq!(fs::read(&dest).unwrap(), *content);
        assert_eq!(connections.load(Ordering::SeqCst), 1);
        assert_eq!(*seen.lock().unwrap(), Protocol::Http2);
    }

    /// What the HTTP/2 test server answers: the range asked for, or the whole file.
    fn range_of(content: &[u8], request: &Request<Incoming>) -> Response<Full<Bytes>> {
        let range = request.headers().get(RANGE).and_then(|value| value.to_str().ok()).and_then(parse_range);
        match range {
            Some((start, end)) if start < content.len() => {
                let end = end.min(content.len() - 1);
                Response::builder()
                    .status(206)
                    .header("content-range", format!("bytes {start}-{end}/{}", content.len()))
                    .body(Full::new(Bytes::copy_from_slice(&content[start..=end])))
                    .unwrap()
            }
            _ => Response::new(Full::new(Bytes::copy_from_slice(content))),
        }
    }

    /// The real thing, over TLS, against a public CDN that speaks HTTP/2 (Cloudflare): the
    /// platform's trust store, ALPN, a range answered. Needs the network, so only when asked:
    /// cargo test real_https -- --ignored
    #[test]
    #[ignore = "needs the network: cargo test real_https -- --ignored"]
    fn real_https_server_is_reached_over_http2() {
        let http = http::client();
        let source = Source { url: "https://cdnjs.cloudflare.com/ajax/libs/jquery/3.7.1/jquery.min.js".into(), authorization: None };
        let response = http.block_on(http.send(source.request(http, Some("bytes=0-1")).unwrap())).expect("an answer");
        assert_eq!(response.version(), hyper::Version::HTTP_2);
        // Cloudflare answers a range with 206 from its cache and with the whole file otherwise.
        assert!(matches!(response.status().as_u16(), 200 | 206), "{}", response.status());
        let scratch = Scratch::new("real");
        let dest = scratch.path("jquery.min.js");
        download(http, &source, &dest, &Streams::new(4), &AtomicBool::new(false), &|_| {}).expect("the file");
        let text = fs::read_to_string(&dest).unwrap();
        assert!(text.starts_with("/*! jQuery v3.7.1"), "{}", &text[..40.min(text.len())]);
    }

    /// A real server, named by the environment: FINCHGRAM_BENCH_URL (the WebDAV address),
    /// FINCHGRAM_BENCH_USER, FINCHGRAM_BENCH_PASSWORD, FINCHGRAM_BENCH_FILE (the file's path on
    /// the server, "/Movies/film.mkv") and FINCHGRAM_BENCH_STREAMS (12 unless set). Prints the
    /// speed every two seconds and the engine's log, leaves nothing behind:
    /// cargo test bench_real_server -- --ignored --nocapture
    #[test]
    #[ignore = "needs a server: cargo test bench_real_server -- --ignored --nocapture"]
    fn bench_real_server() {
        use crate::downloads::webdav::Server;
        let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
        let (Some(url), Some(file)) = (var("FINCHGRAM_BENCH_URL"), var("FINCHGRAM_BENCH_FILE")) else {
            eprintln!("no server given");
            return;
        };
        let server = Server { url: Server::normalized_url(&url), username: var("FINCHGRAM_BENCH_USER").unwrap_or_default(), password: var("FINCHGRAM_BENCH_PASSWORD").unwrap_or_default() };
        let count: usize = var("FINCHGRAM_BENCH_STREAMS").and_then(|value| value.parse().ok()).unwrap_or(DEFAULT_CONNECTIONS);
        let streams = Streams::new(count);
        let segments: Vec<String> = file.trim_matches('/').split('/').map(str::to_string).collect();
        let (name, folder) = segments.split_last().expect("a file name");
        let source = Source { url: server.file_url(folder, name), authorization: server.authorization() };
        let scratch = Scratch::new("bench");
        let dest = scratch.path(name);
        let started = Instant::now();
        let last = Mutex::new((Instant::now(), 0u64));
        let result = download(http::client(), &source, &dest, &streams, &AtomicBool::new(false), &|progress| {
            let mut last = last.lock().unwrap();
            if last.0.elapsed() >= Duration::from_secs(2) {
                let rate = progress.done.saturating_sub(last.1) as f64 / 1048576.0 / last.0.elapsed().as_secs_f64();
                eprintln!("{:6.1} s {:8.1} MB {:6.1} MB/s {}", started.elapsed().as_secs_f64(), progress.done as f64 / 1048576.0, rate, progress.protocol.name());
                *last = (Instant::now(), progress.done);
            }
        });
        let seconds = started.elapsed().as_secs_f64();
        let bytes = fs::metadata(&dest).map(|meta| meta.len()).unwrap_or(0);
        eprintln!("result {:?}: {:.1} MB in {seconds:.1} s = {:.1} MB/s on average, {count} streams", result.as_ref().map(|_| ()), bytes as f64 / 1048576.0, bytes as f64 / 1048576.0 / seconds.max(0.001));
        assert!(result.is_ok(), "{result:?}");
    }

    /// A server that answers every request for the file with `status`, `content_range` and `body`,
    /// whatever was asked for, for as long as the test runs.
    fn misanswering_server(status: &'static str, content_range: &'static str, body: &'static [u8]) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
        let url = format!("http://{}/file", listener.local_addr().expect("the port"));
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut head = Vec::new();
                let mut byte = [0u8];
                while !head.ends_with(b"\r\n\r\n") && std::io::Read::read(&mut stream, &mut byte).is_ok_and(|n| n == 1) {
                    head.push(byte[0]);
                }
                let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Range: {content_range}\r\nConnection: close\r\n\r\n", body.len());
                let _ = stream.write_all(body);
            }
        });
        url
    }

    #[test]
    fn an_answer_for_other_bytes_than_asked_is_not_taken() {
        // Asked for the bytes from 0, answered with bytes 2-3 of 4 (and a 4-byte body): a cache or
        // a proxy gone wrong. Nothing of it is written as the file.
        let scratch = Scratch::new("misranged");
        let dest = scratch.path("sample.bin");
        let url = misanswering_server("206 Partial Content", "bytes 2-3/4", b"BBBB");
        let outcome = download(http::client(), &Source { url, authorization: None }, &dest, &Streams::new(1), &AtomicBool::new(false), &|_| {});
        assert!(matches!(&outcome, Err(Error::Protocol(why)) if why.contains("from 0")), "{outcome:?}");
        assert!(!dest.exists());
        // A Content-Range that cannot be read, and one whose body is not as long as it says.
        let url = misanswering_server("206 Partial Content", "bytes=0-3/4", b"BBBB");
        let outcome = download(http::client(), &Source { url, authorization: None }, &scratch.path("odd.bin"), &Streams::new(1), &AtomicBool::new(false), &|_| {});
        assert!(matches!(&outcome, Err(Error::Protocol(why)) if why.contains("cannot be read")), "{outcome:?}");
        let url = misanswering_server("206 Partial Content", "bytes 0-1/4", b"BBBB");
        let outcome = download(http::client(), &Source { url, authorization: None }, &scratch.path("short.bin"), &Streams::new(1), &AtomicBool::new(false), &|_| {});
        assert!(matches!(&outcome, Err(Error::Protocol(why)) if why.contains("Content-Length")), "{outcome:?}");
    }

    #[test]
    fn a_part_file_that_is_a_link_is_not_written_through() {
        let scratch = Scratch::new("linked");
        let victim = scratch.path("victim");
        fs::write(&victim, b"original").expect("the victim");
        let dest = scratch.path("film.mkv");
        std::os::unix::fs::symlink(&victim, with_suffix(&dest, ".part")).expect("a link");
        let server = serve(pattern(100_000), true, None);
        let outcome = fetch(&server, &dest, 2);
        assert!(matches!(&outcome, Err(Error::Disk(why)) if why.contains("symbolic") || why.contains("symlink") || why.contains("not a regular file") || why.contains("Too many levels")), "{outcome:?}");
        assert_eq!(fs::read(&victim).expect("the victim"), b"original");
        assert!(!dest.exists());
    }

    #[test]
    fn a_removed_downloads_partial_files_go_and_nothing_else() {
        let scratch = Scratch::new("partials");
        let dest = scratch.path("film.mkv");
        fs::write(with_suffix(&dest, ".part"), b"half").unwrap();
        fs::write(with_suffix(&dest, ".part.json"), b"{}").unwrap();
        fs::write(scratch.path("notes.txt"), b"kept").unwrap();
        remove_partials(&dest);
        assert!(!with_suffix(&dest, ".part").exists() && !with_suffix(&dest, ".part.json").exists());
        assert!(scratch.path("notes.txt").exists());
        // A folder: every partial file under it, however deep; the finished ones stay.
        let folder = scratch.path("Season 1");
        fs::create_dir_all(folder.join("Extras")).unwrap();
        fs::write(folder.join("e1.mkv.part"), b"half").unwrap();
        fs::write(folder.join("e1.mkv.part.json"), b"{}").unwrap();
        fs::write(folder.join("e2.mkv"), b"whole").unwrap();
        fs::write(folder.join("Extras").join("x.mkv.part"), b"half").unwrap();
        remove_partials(&folder);
        assert!(!folder.join("e1.mkv.part").exists() && !folder.join("e1.mkv.part.json").exists() && !folder.join("Extras").join("x.mkv.part").exists());
        assert!(folder.join("e2.mkv").exists());
    }

    #[test]
    fn a_taken_name_counts_up() {
        let scratch = Scratch::new("names");
        assert_eq!(free_path(&scratch.path("film.mkv")), scratch.path("film.mkv"));
        fs::write(scratch.path("film.mkv"), b"").unwrap();
        assert_eq!(free_path(&scratch.path("film.mkv")), scratch.path("film (2).mkv"));
        fs::write(scratch.path("film (2).mkv"), b"").unwrap();
        assert_eq!(free_path(&scratch.path("film.mkv")), scratch.path("film (3).mkv"));
        fs::write(scratch.path("README"), b"").unwrap();
        assert_eq!(free_path(&scratch.path("README")), scratch.path("README (2)"));
        fs::write(scratch.path(".hidden"), b"").unwrap();
        assert_eq!(free_path(&scratch.path(".hidden")), scratch.path(".hidden (2)"));
        assert_eq!(with_suffix(Path::new("/a/film.mkv"), ".part"), PathBuf::from("/a/film.mkv.part"));
    }

    #[test]
    fn every_chunk_is_eight_megabytes() {
        assert_eq!(chunk_size(100), CHUNK_SIZE);
        assert_eq!(chunk_size(20 << 30), CHUNK_SIZE);
    }

    #[test]
    fn the_last_chunk_is_shorter() {
        let total = 2 * CHUNK_SIZE + 123;
        assert_eq!(chunk_length(0, total, CHUNK_SIZE), CHUNK_SIZE);
        assert_eq!(chunk_length(1, total, CHUNK_SIZE), CHUNK_SIZE);
        assert_eq!(chunk_length(2, total, CHUNK_SIZE), 123);
        assert_eq!(chunk_length(3, total, CHUNK_SIZE), 0);
    }
}
