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
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
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

/// Streams per file unless settings say otherwise. vget's 12 was the start; measured against
/// PikPak's CDN, whose connections each give 1 to 10 MB/s, 24 streams go half as fast again.
pub const DEFAULT_CONNECTIONS: usize = 24;
/// The most streams settings may ask for.
pub const MOST_CONNECTIONS: usize = 64;
/// A chunk: 8 MB for a small file, small enough that a failed one is cheap to fetch again; up to
/// 32 MB for a large one, since a stream sits idle while the server works out each request
/// (PikPak takes half a second), and the fewer chunks the less of that.
const CHUNK_SIZE: u64 = 8 * 1024 * 1024;
const LARGEST_CHUNK: u64 = 32 * 1024 * 1024;
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
/// The tail: once nothing is pending, an idle stream takes most of the longest part still on its
/// way, when that is at least this much. Without it the last chunks on the slowest connections
/// (1 MB/s, or 0.2, on a bad CDN node) kept a 1 GB file waiting ten seconds after the rest.
const LEAST_STOLEN: u64 = 1024 * 1024;
/// A crawling stream: one that, over the last `WATCH_SPAN`, brought less than a quarter of what
/// the median stream did (the median itself at least 1 MB/s), with at least 2 MB still to come.
/// Its connection is dropped and the rest asked for anew, which on a CDN lands on another node;
/// at most twice per stretch. PikPak's nodes differ tenfold, and a slow one held its chunk for
/// ten seconds while the others raced.
const WATCH_SPAN: Duration = Duration::from_secs(3);
const CRAWL_RATIO: u64 = 4;
const MEDIAN_FLOOR: u64 = 1024 * 1024;
const LEAST_REROLLED: u64 = 2 * 1024 * 1024;
const MOST_REROLLS: usize = 2;

/// How many streams a transfer may use: `count` per file, and a budget shared by every file of
/// a folder being downloaded at once, so that several files together use no more than one.
#[derive(Clone)]
pub struct Streams {
    pub count: usize,
    budget: Arc<Semaphore>,
    /// Whether crawling streams are replaced (always, but for measuring without).
    pub watch: bool,
}

impl Streams {
    pub fn new(count: usize) -> Streams {
        let count = count.clamp(1, MOST_CONNECTIONS);
        Streams { count, budget: Arc::new(Semaphore::new(count)), watch: true }
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

/// How far a transfer is: reported to the caller every 50 ms while bytes come.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
    pub protocol: Protocol,
}

/// Where a file is, and the account to get it with (webdav::Server::authorization).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub url: String,
    pub authorization: Option<String>,
}

impl Source {
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
}

/// What the workers of one file share.
struct Shared {
    total: u64,
    chunk_size: u64,
    /// Streams in flight across every file of the transfer (Streams::budget).
    budget: Arc<Semaphore>,
    /// The chunks still to fetch.
    pending: Mutex<VecDeque<usize>>,
    /// Bytes handed to the writer so far, the done chunks of a resumed file counted.
    bytes: AtomicU64,
    /// A worker failed, or the user stopped the file: the others stop.
    abort: AtomicBool,
    failed: Mutex<Option<Error>>,
    /// The parts of chunks on their way, for the tail.
    parts: Mutex<Vec<Part>>,
    /// How many parts of each chunk are still on their way: the chunk is done when the last is.
    parts_left: Mutex<HashMap<usize, usize>>,
}

/// A stretch of a chunk one stream fetches: from `offset`, which moves as bytes are written, up
/// to `stop`, which an idle stream moves down when it takes the rest.
struct Part {
    chunk: usize,
    offset: Arc<AtomicU64>,
    stop: Arc<AtomicU64>,
    /// Set by the watch in `multi`: drop this connection and ask for the rest anew.
    reroll: Arc<AtomicBool>,
    rerolls: Arc<AtomicUsize>,
}

impl Part {
    fn new(chunk: usize, from: u64, to: u64) -> Part {
        Part {
            chunk,
            offset: Arc::new(AtomicU64::new(from)),
            stop: Arc::new(AtomicU64::new(to)),
            reroll: Arc::new(AtomicBool::new(false)),
            rerolls: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// What tells this part apart, for the watch.
    fn key(&self) -> usize {
        Arc::as_ptr(&self.offset) as usize
    }

    fn remaining(&self) -> u64 {
        self.stop.load(Ordering::Relaxed).saturating_sub(self.offset.load(Ordering::Relaxed))
    }

    fn same(&self, other: &Part) -> bool {
        Arc::ptr_eq(&self.offset, &other.offset)
    }
}

impl Clone for Part {
    fn clone(&self) -> Part {
        Part { chunk: self.chunk, offset: self.offset.clone(), stop: self.stop.clone(), reroll: self.reroll.clone(), rerolls: self.rerolls.clone() }
    }
}

/// Which of the parts on their way crawl: each is (bytes per second over the watch span, bytes
/// still to come, times rerolled already). The median rate of all of them is the measure; with
/// fewer than three parts, or a median under the floor (the server is slow for everyone), none.
fn crawling(parts: &[(u64, u64, usize)]) -> Vec<bool> {
    if parts.len() < 3 {
        return vec![false; parts.len()];
    }
    let mut rates: Vec<u64> = parts.iter().map(|(rate, _, _)| *rate).collect();
    rates.sort_unstable();
    let median = rates[rates.len() / 2];
    if median < MEDIAN_FLOOR {
        return vec![false; parts.len()];
    }
    parts.iter().map(|(rate, remaining, rerolls)| *rate < median / CRAWL_RATIO && *remaining >= LEAST_REROLLED && *rerolls < MOST_REROLLS).collect()
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
}

/// What the streams hand the writer thread: bytes for an offset, or word that a chunk's last
/// byte has been handed over, answered once it is on disk.
enum Write {
    Data { offset: u64, data: Bytes },
    Done { index: usize, ack: oneshot::Sender<()> },
}

/// Ask the server about the file: its size, and whether it does ranges.
pub fn probe(http: &Http, source: &Source) -> Result<Probe, Error> {
    http.block_on(probe_once(http, source))
}

async fn probe_once(http: &Http, source: &Source) -> Result<Probe, Error> {
    let started = Instant::now();
    let mut response = http.send(source.request(http, Some("bytes=0-1"))?).await?;
    let protocol = Protocol::of(response.version());
    log::line(format!("probe {}: {} {} in {} ms", source.url, protocol.name(), response.status().as_u16(), started.elapsed().as_millis()));
    match response.status().as_u16() {
        206 => {
            let total = response.headers().get("content-range").and_then(|value| value.to_str().ok()).and_then(content_range_total);
            // The two bytes, read to the body's end: a connection with its body read is kept.
            drain(response.body_mut(), 64).await;
            Ok(Probe { size: total, ranges: total.is_some(), protocol })
        }
        // 416: nothing to range over, the file is empty.
        416 => Ok(Probe { size: Some(0), ranges: true, protocol }),
        200 => Ok(Probe { size: content_length(&response), ranges: false, protocol }),
        code => Err(status_error(code)),
    }
}

/// The whole size out of "bytes 0-1/12345"; none for "bytes 0-1/*".
fn content_range_total(header: &str) -> Option<u64> {
    header.trim().rsplit('/').next()?.trim().parse().ok()
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

/// Fetch the file at `source` into `dest`, over up to `connections` streams, reporting (bytes
/// done, bytes in all) to `progress` as it goes. A `dest` that is taken gets " (2)" and so on; the
/// path the file got is returned. `cancel` stops it and leaves the `.part` files for a later try.
/// Blocks: call it on a thread of the file's own.
pub fn download(http: &Http, source: &Source, dest: &Path, streams: &Streams, cancel: &AtomicBool, progress: &(dyn Fn(Progress) + Sync)) -> Result<PathBuf, Error> {
    if let Some(folder) = dest.parent() {
        fs::create_dir_all(folder).map_err(disk)?;
    }
    let part = with_suffix(dest, ".part");
    let state_path = with_suffix(dest, ".part.json");
    let started = Instant::now();
    let probe = http.block_on(probe_with_retries(http, source, cancel))?;
    let protocol = probe.protocol;
    match probe {
        Probe { size: Some(total), ranges: true, .. } if total > 0 => {
            http.block_on(multi(http, source, &part, &state_path, total, streams, cancel, &|done, total| progress(Progress { done, total, protocol })))?;
        }
        Probe { size: Some(0), .. } => {
            File::create(&part).map_err(disk)?;
            progress(Progress { done: 0, total: 0, protocol });
        }
        Probe { size, .. } => http.block_on(single(http, source, &part, size, cancel, &|done, total| progress(Progress { done, total, protocol })))?,
    }
    let _ = fs::remove_file(&state_path);
    let target = free_path(dest);
    fs::rename(&part, &target).map_err(disk)?;
    log::line(format!("done {}: {} in {:.1} s", source.url, size_words(fs::metadata(&target).map(|meta| meta.len()).unwrap_or(0)), started.elapsed().as_secs_f64()));
    Ok(target)
}

/// "12.3 MB", for the log.
fn size_words(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

/// The chunk a file of `total` bytes is cut into: 8 MB, growing to 32 MB for a large file.
fn chunk_size(total: u64) -> u64 {
    (total / 96).clamp(CHUNK_SIZE, LARGEST_CHUNK)
}

/// Bytes of `dest` that are there from an earlier try: the done chunks of its `.part` file.
pub fn kept(dest: &Path) -> u64 {
    let Some(state) = read_state(&with_suffix(dest, ".part.json")) else { return 0 };
    if state.chunk_size == 0 || state.done.len() != state.chunks {
        return 0;
    }
    state.done.iter().enumerate().filter(|(_, done)| **done).map(|(index, _)| chunk_length(index, state.size, state.chunk_size)).sum()
}

/// Several streams: the file in chunks, each worker taking the next chunk until none is left,
/// the writer thread putting the bytes where they belong.
#[allow(clippy::too_many_arguments)]
async fn multi(
    http: &Http,
    source: &Source,
    part: &Path,
    state_path: &Path,
    total: u64,
    streams: &Streams,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, u64),
) -> Result<(), Error> {
    let chunk = chunk_size(total);
    let chunks = usize::try_from(total.div_ceil(chunk)).unwrap_or(usize::MAX);
    let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(part).map_err(disk)?;
    // An earlier try of the same file goes on; anything else starts afresh.
    let earlier = read_state(state_path)
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
    let pending: VecDeque<usize> = (0..chunks).filter(|index| !done[*index]).collect();
    let shared = Arc::new(Shared {
        total,
        chunk_size: chunk,
        budget: streams.budget.clone(),
        pending: Mutex::new(pending),
        bytes: AtomicU64::new(already),
        abort: AtomicBool::new(false),
        failed: Mutex::new(None),
        parts: Mutex::new(Vec::new()),
        parts_left: Mutex::new(HashMap::new()),
    });
    progress(already, total);
    let (sender, receiver) = mpsc::channel::<Write>(WRITES_IN_FLIGHT);
    let writer = Writer::start(file, state_path.to_path_buf(), total, chunk, done, receiver, shared.clone());
    let workers = streams.count.min(chunks.max(1));
    log::line(format!("start {}: {}, {} chunks of {}, {} streams, {} kept", source.url, size_words(total), chunks, size_words(chunk), workers, size_words(already)));
    let source = Arc::new(source.clone());
    let mut set = tokio::task::JoinSet::new();
    for _ in 0..workers {
        set.spawn_on(worker(http.clone(), source.clone(), shared.clone(), sender.clone()), http.handle());
    }
    drop(sender);
    // Progress every tick, the user's cancel passed on, and once a second a look at which stream
    // crawls, until the last worker is done.
    let mut ticker = tokio::time::interval(PROGRESS_TICK);
    let mut last = already;
    let mut watch: HashMap<usize, VecDeque<(Instant, u64)>> = HashMap::new();
    let mut looked = Instant::now();
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if cancel.load(Ordering::Relaxed) {
                    shared.abort.store(true, Ordering::Relaxed);
                }
                let now = shared.bytes.load(Ordering::Relaxed);
                if now != last {
                    last = now;
                    progress(now, total);
                }
                let parts = shared.parts.lock().unwrap().clone();
                let moment = Instant::now();
                watch.retain(|key, _| parts.iter().any(|part| part.key() == *key));
                for part in &parts {
                    let history = watch.entry(part.key()).or_default();
                    history.push_back((moment, part.offset.load(Ordering::Relaxed)));
                    while history.len() > 2 && history.front().is_some_and(|(at, _)| moment.duration_since(*at) > WATCH_SPAN) {
                        history.pop_front();
                    }
                }
                if streams.watch && looked.elapsed() >= Duration::from_secs(1) {
                    looked = moment;
                    let watched: Vec<(&Part, u64)> = parts
                        .iter()
                        .filter_map(|part| {
                            let history = watch.get(&part.key())?;
                            let (first_at, first) = *history.front()?;
                            let (last_at, last) = *history.back()?;
                            let seconds = last_at.duration_since(first_at).as_secs_f64();
                            (seconds >= WATCH_SPAN.as_secs_f64() * 0.9).then(|| (part, ((last - first) as f64 / seconds) as u64))
                        })
                        .collect();
                    let measured: Vec<(u64, u64, usize)> = watched.iter().map(|(part, rate)| (*rate, part.remaining(), part.rerolls.load(Ordering::Relaxed))).collect();
                    for ((part, rate), crawls) in watched.iter().zip(crawling(&measured)) {
                        if crawls && !part.reroll.swap(true, Ordering::Relaxed) {
                            log::line(format!("chunk {}: crawling at {:.1} MB/s with {} to come: another connection", part.chunk, *rate as f64 / (1024.0 * 1024.0), size_words(part.remaining())));
                        }
                    }
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
    progress(shared.bytes.load(Ordering::Relaxed), total);
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

impl Writer {
    fn start(file: File, state_path: PathBuf, total: u64, chunk_size: u64, mut done: Vec<bool>, mut receiver: mpsc::Receiver<Write>, shared: Arc<Shared>) -> Writer {
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
                            write_state(&state_path, total, chunk_size, &done);
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

/// The next chunk; when none is pending, three quarters of the longest part still on its way
/// (the stream that has it is slow, or it would not be the longest), if that is worth a request
/// of its own.
fn next_part(shared: &Shared) -> Option<Part> {
    let part = match shared.pending.lock().unwrap().pop_front() {
        Some(index) => {
            let start = index as u64 * shared.chunk_size;
            Part::new(index, start, start + chunk_length(index, shared.total, shared.chunk_size))
        }
        None => {
            let parts = shared.parts.lock().unwrap();
            let longest = parts.iter().max_by_key(|part| part.remaining())?;
            let remaining = longest.remaining();
            if remaining < LEAST_STOLEN {
                return None;
            }
            let stop = longest.stop.load(Ordering::Relaxed);
            let mid = stop - remaining * 3 / 4;
            longest.stop.store(mid, Ordering::Relaxed);
            log::line(format!("chunk {}: an idle stream takes its last {}", longest.chunk, size_words(stop - mid)));
            Part::new(longest.chunk, mid, stop)
        }
    };
    shared.parts.lock().unwrap().push(part.clone());
    *shared.parts_left.lock().unwrap().entry(part.chunk).or_insert(0) += 1;
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
        match fetch_range(http, source, part, shared, writer).await {
            Ok(true) => break,
            // Dropped as a crawler: the rest anew, now, over another connection.
            Ok(false) => {
                part.rerolls.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            // The disk is the one thing trying again will not help. Everything else may pass next
            // time, as vget has it: a connection that drops, a 429 or a 5xx from a busy server, a
            // 404 from a CDN node that does not have the file yet (PikPak does that), a 200 where a
            // 206 was due.
            Err(err @ Error::Disk(_)) => return Err(err),
            Err(err) => {
                attempts = if part.offset.load(Ordering::Relaxed) > from { 1 } else { attempts + 1 };
                log::line(format!("chunk {} of {}: {err}; try {attempts} of {ATTEMPTS}", part.chunk, source.url));
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

/// The bytes of `part` from its offset to its stop, handed to the writer for where they belong;
/// the offset moves as they go. The stop may move down meanwhile (an idle stream took the rest):
/// the body is then left unread, and the connection closed, which is cheaper than the wait. True
/// when the part is complete; false when the watch had the connection dropped as a crawler.
async fn fetch_range(http: &Http, source: &Source, part: &Part, shared: &Shared, writer: &mpsc::Sender<Write>) -> Result<bool, Error> {
    let from = part.offset.load(Ordering::Relaxed);
    let end = part.stop.load(Ordering::Relaxed) - 1;
    let started = Instant::now();
    let mut response = http.send(source.request(http, Some(&format!("bytes={from}-{end}")))?).await?;
    let answered = started.elapsed();
    match response.status().as_u16() {
        206 => {}
        200 => return Err(Error::Protocol("the server ignored the range it said it would answer".into())),
        code => return Err(status_error(code)),
    }
    let body = response.body_mut();
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
            return Ok(true);
        }
        if part.reroll.swap(false, Ordering::Relaxed) {
            return Ok(false);
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

/// The probe, tried again like a chunk is: a server that answers 404 or 5xx once is asked again.
async fn probe_with_retries(http: &Http, source: &Source, cancel: &AtomicBool) -> Result<Probe, Error> {
    let mut attempts: u32 = 0;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        match probe_once(http, source).await {
            Ok(probe) => return Ok(probe),
            Err(err) => {
                attempts += 1;
                if attempts >= ATTEMPTS {
                    return Err(err);
                }
                wait(backoff(attempts), cancel).await?;
            }
        }
    }
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

/// One stream, from the start: for a server that does not do ranges. Nothing to go on from when
/// it fails.
async fn single(http: &Http, source: &Source, part: &Path, size: Option<u64>, cancel: &AtomicBool, progress: &dyn Fn(u64, u64)) -> Result<(), Error> {
    let mut response = http.send(source.request(http, None)?).await?;
    if !response.status().is_success() {
        return Err(status_error(response.status().as_u16()));
    }
    let total = content_length(&response).or(size).unwrap_or(0);
    let mut file = File::create(part).map_err(disk)?;
    let body = response.body_mut();
    let mut done: u64 = 0;
    let mut reported = tokio::time::Instant::now();
    progress(0, total);
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
        if reported.elapsed() >= PROGRESS_TICK {
            progress(done, total.max(done));
            reported = tokio::time::Instant::now();
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
    }
    progress(done, total.max(done));
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

fn write_state(path: &Path, total: u64, chunk_size: u64, done: &[bool]) {
    let state = State { size: total, chunk_size, chunks: done.len(), done: done.to_vec() };
    match serde_json::to_string(&state) {
        Ok(text) => {
            if let Err(err) = fs::write(path, text) {
                eprintln!("downloads: cannot write {}: {err}", path.display());
            }
        }
        Err(err) => eprintln!("downloads: cannot describe {}: {err}", path.display()),
    }
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
    /// `probe_failures` probes.
    struct FileServer {
        url: String,
        connections: Arc<AtomicUsize>,
        requests: Arc<Mutex<Vec<String>>>,
        failures: Arc<AtomicUsize>,
        missing: Arc<AtomicUsize>,
        probe_failures: Arc<AtomicUsize>,
        /// The next `slow` chunk requests are answered 64 KB every 20 ms.
        slow: Arc<AtomicUsize>,
    }

    fn serve(content: Vec<u8>, ranges: bool, authorization: Option<&str>) -> FileServer {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
        let port = listener.local_addr().expect("the port").port();
        let connections = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let failures = Arc::new(AtomicUsize::new(0));
        let missing = Arc::new(AtomicUsize::new(0));
        let probe_failures = Arc::new(AtomicUsize::new(0));
        let slow = Arc::new(AtomicUsize::new(0));
        let content = Arc::new(content);
        let authorization = authorization.map(str::to_string);
        let counters = Arc::new(Counters { failures: failures.clone(), missing: missing.clone(), probe_failures: probe_failures.clone(), slow: slow.clone() });
        let (opened, seen) = (connections.clone(), requests.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                opened.fetch_add(1, Ordering::SeqCst);
                let (content, seen, counters, authorization) = (content.clone(), seen.clone(), counters.clone(), authorization.clone());
                std::thread::spawn(move || answer(stream, &content, ranges, authorization.as_deref(), &seen, &counters));
            }
        });
        FileServer { url: format!("http://127.0.0.1:{port}/film.mkv"), connections, requests, failures, missing, probe_failures, slow }
    }

    /// How many of the next requests the server turns away, and how; how many it answers slowly.
    struct Counters {
        failures: Arc<AtomicUsize>,
        missing: Arc<AtomicUsize>,
        probe_failures: Arc<AtomicUsize>,
        slow: Arc<AtomicUsize>,
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
            let probe = range.as_deref() == Some("bytes=0-1");
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
            if stream.write_all(head.as_bytes()).is_err() {
                return;
            }
            if chunk && take(&counters.slow) {
                for piece in body.chunks(64 * 1024) {
                    if stream.write_all(piece).is_err() {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            } else if stream.write_all(body).is_err() {
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
        assert_eq!(content_range_total("bytes 0-1/12345"), Some(12345));
        assert_eq!(content_range_total("bytes 0-1/*"), None);
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
        assert_eq!(requests[0], "bytes=0-1");
        assert!(requests.len() >= 4, "{requests:?}");
        assert!(covers(&requests[1..], content.len()), "every byte asked for: {requests:?}");
    }

    /// Whether `ranges` ("bytes=a-b"), together, ask for every byte of a file of `len` bytes
    /// (an idle stream may take the tail of another's chunk, so they may overlap).
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
    fn an_idle_stream_takes_the_tail_of_a_slow_one() {
        // Two chunks over four streams, the first chunk's connection slow: the idle streams take
        // its rest, most of it at a time, instead of waiting for it.
        let content = pattern(2 * CHUNK_SIZE as usize + 5);
        let server = serve(content.clone(), true, None);
        server.slow.store(1, Ordering::SeqCst);
        let scratch = Scratch::new("tail");
        let dest = scratch.path("film.mkv");
        let started = Instant::now();
        fetch(&server, &dest, 4).expect("the file");
        assert_eq!(fs::read(&dest).unwrap(), content);
        let requests = server.requests.lock().unwrap().clone();
        assert!(requests.len() > 3, "the tail was taken: {requests:?}");
        assert!(covers(&requests[1..], content.len()), "{requests:?}");
        // The slow connection alone would have taken 8 MB at 64 KB per 20 ms: 2.6 s.
        assert!(started.elapsed() < Duration::from_millis(2000), "{:?}", started.elapsed());
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
        let requests = server.requests.lock().unwrap().len();
        assert!((13..=16).contains(&requests), "{requests} requests for 12 chunks");
        let connections = server.connections.load(Ordering::SeqCst);
        assert!(connections <= 6, "{connections} connections for 12 chunks over 2");
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
        write_state(&with_suffix(&dest, ".part.json"), content.len() as u64, CHUNK_SIZE, &[true, false, false]);
        assert_eq!(kept(&dest), CHUNK_SIZE);
        let got = fetch(&server, &dest, 2).expect("the file");
        assert_eq!(got, dest);
        assert_eq!(fs::read(&dest).unwrap(), content);
        let requests = server.requests.lock().unwrap().clone();
        let lowest = requests[1..].iter().filter_map(|range| parse_range(range)).map(|(start, _)| start).min();
        assert_eq!(lowest, Some(CHUNK_SIZE as usize), "the first chunk is not fetched again: {requests:?}");
        assert!(requests.len() >= 3);
        assert_eq!(kept(&dest), 0);
    }

    #[test]
    fn one_stream_when_the_server_does_no_ranges() {
        let content = pattern(100_000);
        let server = serve(content.clone(), false, None);
        let scratch = Scratch::new("single");
        let dest = scratch.path("film.mkv");
        fetch(&server, &dest, 4).expect("the file");
        assert_eq!(fs::read(&dest).unwrap(), content);
        assert_eq!(*server.requests.lock().unwrap(), vec!["bytes=0-1".to_string(), String::new()]);
    }

    #[test]
    fn a_failed_chunk_is_tried_again() {
        let content = pattern(100_000);
        let server = serve(content.clone(), true, None);
        server.failures.store(2, Ordering::SeqCst);
        let scratch = Scratch::new("retry");
        let dest = scratch.path("film.mkv");
        fetch(&server, &dest, 4).expect("the file");
        assert_eq!(fs::read(&dest).unwrap(), content);
        // The probe, two tries answered 429, the one that worked.
        assert_eq!(server.requests.lock().unwrap().len(), 4);
    }

    #[test]
    fn a_404_from_a_node_without_the_file_yet_passes() {
        // PikPak's CDN answers 404 for a moment to a range of a file it is still fetching.
        let content = pattern(100_000);
        let server = serve(content.clone(), true, None);
        server.missing.store(1, Ordering::SeqCst);
        let scratch = Scratch::new("missing");
        let dest = scratch.path("film.mkv");
        fetch(&server, &dest, 4).expect("the file");
        assert_eq!(fs::read(&dest).unwrap(), content);
        // The probe, the 404, the one that worked.
        assert_eq!(server.requests.lock().unwrap().len(), 3);
    }

    #[test]
    fn the_probe_is_tried_again_too() {
        let content = pattern(100_000);
        let server = serve(content.clone(), true, None);
        server.probe_failures.store(1, Ordering::SeqCst);
        let scratch = Scratch::new("probe-retry");
        let dest = scratch.path("film.mkv");
        fetch(&server, &dest, 4).expect("the file");
        assert_eq!(fs::read(&dest).unwrap(), content);
        // Two probes, one chunk.
        assert_eq!(server.requests.lock().unwrap().len(), 3);
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
        assert_eq!(kept(&dest), 0);
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
        let mut streams = Streams::new(count);
        streams.watch = var("FINCHGRAM_BENCH_NO_WATCH").is_none();
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
        eprintln!("result {:?}: {:.1} MB in {seconds:.1} s = {:.1} MB/s on average, {count} streams, watch {}", result.as_ref().map(|_| ()), bytes as f64 / 1048576.0, bytes as f64 / 1048576.0 / seconds.max(0.001), streams.watch);
        assert!(result.is_ok(), "{result:?}");
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
    fn a_crawler_is_told_from_the_rest() {
        const MB: u64 = 1024 * 1024;
        // Eight streams at 6 to 9 MB/s and one at 1: the one crawls, as long as it has 2 MB to
        // come and has not been rerolled twice.
        let mut parts: Vec<(u64, u64, usize)> = (0..8).map(|i| ((6 + i % 4) * MB, 5 * MB, 0)).collect();
        parts.push((MB, 5 * MB, 0));
        assert_eq!(crawling(&parts).iter().filter(|crawls| **crawls).count(), 1);
        assert!(crawling(&parts)[8]);
        parts[8] = (MB, MB, 0);
        assert!(!crawling(&parts)[8], "too little left to be worth a request");
        parts[8] = (MB, 5 * MB, MOST_REROLLS);
        assert!(!crawling(&parts)[8], "rerolled enough");
        // Everyone slow: nobody crawls. Two streams: nothing to compare.
        let slow: Vec<(u64, u64, usize)> = (0..6).map(|i| (100 * 1024 + i, 5 * MB, 0)).collect();
        assert!(crawling(&slow).iter().all(|crawls| !crawls));
        assert!(crawling(&[(10 * MB, 5 * MB, 0), (MB, 5 * MB, 0)]).iter().all(|crawls| !crawls));
    }

    #[test]
    fn chunks_grow_with_the_file() {
        assert_eq!(chunk_size(100), CHUNK_SIZE);
        assert_eq!(chunk_size(1 << 30), (1u64 << 30) / 96);
        assert_eq!(chunk_size(20 << 30), LARGEST_CHUNK);
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
