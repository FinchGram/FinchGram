//! The transfer engine: one file over several connections at once, as vget does it. A probe asks
//! for the first two bytes with a Range header: a 206 answer gives the file's size and says the
//! server does ranges, so the file is cut into chunks of 8 MB, fetched by a pool of workers (12
//! connections by default) and written into a pre-sized `.part` file at their offsets. Which chunks
//! are done is kept in `.part.json`, so a stopped download goes on where it was. A chunk that fails
//! is fetched again from the last byte written, up to ten times with a growing pause; a server
//! that answers 200 to a range gets one connection. Done, the `.part` file takes its name.

use std::collections::VecDeque;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// How many connections a file is fetched over, unless the settings say otherwise.
pub const DEFAULT_CONNECTIONS: usize = 12;
/// The most the settings may ask for.
pub const MOST_CONNECTIONS: usize = 32;
/// A chunk: what one connection fetches at a time.
const CHUNK_SIZE: u64 = 8 * 1024 * 1024;
/// What is read from a connection before it is written.
const BUFFER_SIZE: usize = 1024 * 1024;
/// How often a chunk is tried before the file is given up; the count starts again whenever a try
/// got somewhere.
const ATTEMPTS: u32 = 10;
const FIRST_BACKOFF: Duration = Duration::from_millis(500);
const LONGEST_BACKOFF: Duration = Duration::from_secs(8);
/// How often progress is reported.
const PROGRESS_TICK: Duration = Duration::from_millis(50);

/// Where a file is, and the account to get it with (webdav::Server::authorization).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub url: String,
    pub authorization: Option<String>,
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
    /// The server did not do what it said it would (a range ignored).
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

impl From<ureq::Error> for Error {
    fn from(err: ureq::Error) -> Self {
        match err {
            ureq::Error::StatusCode(401) => Error::Unauthorized,
            ureq::Error::StatusCode(403) => Error::Forbidden,
            ureq::Error::StatusCode(code) => Error::Status(code),
            ureq::Error::BadUri(uri) => Error::Protocol(format!("bad address {uri}")),
            other => Error::Connection(other.to_string()),
        }
    }
}

fn disk(err: std::io::Error) -> Error {
    Error::Disk(err.to_string())
}

/// What the probe found out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Probe {
    /// The file's size, when the server said.
    pub size: Option<u64>,
    /// The server answers a range with that range: several connections can share the file.
    pub ranges: bool,
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
    file: File,
    total: u64,
    state_path: PathBuf,
    /// The chunks still to fetch.
    pending: Mutex<VecDeque<usize>>,
    done: Mutex<Vec<bool>>,
    /// Bytes written so far, the done chunks of a resumed file counted.
    bytes: AtomicU64,
    /// A worker failed: the others stop.
    abort: AtomicBool,
    failed: Mutex<Option<Error>>,
}

/// Ask the server about the file: its size, and whether it does ranges.
pub fn probe(agent: &ureq::Agent, source: &Source) -> Result<Probe, Error> {
    let mut request = agent.get(source.url.as_str()).header("Range", "bytes=0-1");
    if let Some(authorization) = &source.authorization {
        request = request.header("Authorization", authorization.as_str());
    }
    match request.call() {
        Ok(mut response) => {
            if response.status().as_u16() == 206 {
                let total = response.headers().get("content-range").and_then(|value| value.to_str().ok()).and_then(content_range_total);
                // The two bytes, read to the body's end: the connection then serves the first
                // chunk. (Not read_to_vec with a limit: ureq's limited reader fails at the limit
                // before it sees the end.) A server that sends more gets its connection closed.
                let mut reader = response.body_mut().as_reader();
                let mut sink = [0u8; 64];
                let mut allowance = sink.len();
                while allowance > 0 && matches!(reader.read(&mut sink[..allowance]), Ok(count) if count > 0) {
                    allowance -= sink.len().min(allowance);
                }
                Ok(Probe { size: total, ranges: total.is_some() })
            } else {
                Ok(Probe { size: response.body().content_length(), ranges: false })
            }
        }
        // 416: nothing to range over, the file is empty.
        Err(ureq::Error::StatusCode(416)) => Ok(Probe { size: Some(0), ranges: true }),
        Err(err) => Err(err.into()),
    }
}

/// The whole size out of "bytes 0-1/12345"; none for "bytes 0-1/*".
fn content_range_total(header: &str) -> Option<u64> {
    header.trim().rsplit('/').next()?.trim().parse().ok()
}

/// Fetch the file at `source` into `dest`, over up to `connections` connections, reporting
/// (bytes done, bytes in all) to `progress` as it goes. A `dest` that is taken gets " (2)" and so
/// on; the path the file got is returned. `cancel` stops it and leaves the `.part` files for a
/// later try. Blocks: call it on a thread of the file's own.
pub fn download(
    agent: &ureq::Agent,
    source: &Source,
    dest: &Path,
    connections: usize,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, u64),
) -> Result<PathBuf, Error> {
    if let Some(folder) = dest.parent() {
        fs::create_dir_all(folder).map_err(disk)?;
    }
    let part = with_suffix(dest, ".part");
    let state_path = with_suffix(dest, ".part.json");
    let probe = probe_with_retries(agent, source, cancel)?;
    match probe {
        Probe { size: Some(total), ranges: true } if total > 0 => {
            multi(agent, source, &part, &state_path, total, connections, cancel, progress)?;
        }
        Probe { size: Some(0), .. } => {
            File::create(&part).map_err(disk)?;
            progress(0, 0);
        }
        Probe { size, .. } => single(agent, source, &part, size, cancel, progress)?,
    }
    let _ = fs::remove_file(&state_path);
    let target = free_path(dest);
    fs::rename(&part, &target).map_err(disk)?;
    Ok(target)
}

/// Bytes of `dest` that are there from an earlier try: the done chunks of its `.part` file.
pub fn kept(dest: &Path) -> u64 {
    let Some(state) = read_state(&with_suffix(dest, ".part.json")) else { return 0 };
    if state.chunk_size == 0 || state.done.len() != state.chunks {
        return 0;
    }
    state.done.iter().enumerate().filter(|(_, done)| **done).map(|(index, _)| chunk_length(index, state.size, state.chunk_size)).sum()
}

/// Several connections: the file in chunks, each worker taking the next chunk until none is left.
#[allow(clippy::too_many_arguments)]
fn multi(
    agent: &ureq::Agent,
    source: &Source,
    part: &Path,
    state_path: &Path,
    total: u64,
    connections: usize,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64, u64),
) -> Result<(), Error> {
    let chunks = usize::try_from(total.div_ceil(CHUNK_SIZE)).unwrap_or(usize::MAX);
    let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(part).map_err(disk)?;
    // An earlier try of the same file goes on; anything else starts afresh.
    let earlier = read_state(state_path)
        .filter(|state| state.size == total && state.chunk_size == CHUNK_SIZE && state.chunks == chunks && state.done.len() == chunks)
        .filter(|_| file.metadata().map(|meta| meta.len() == total).unwrap_or(false));
    let done = match earlier {
        Some(state) => state.done,
        None => {
            file.set_len(total).map_err(disk)?;
            vec![false; chunks]
        }
    };
    let already: u64 = done.iter().enumerate().filter(|(_, done)| **done).map(|(index, _)| chunk_length(index, total, CHUNK_SIZE)).sum();
    let pending: VecDeque<usize> = (0..chunks).filter(|index| !done[*index]).collect();
    let shared = Shared {
        file,
        total,
        state_path: state_path.to_path_buf(),
        pending: Mutex::new(pending),
        done: Mutex::new(done),
        bytes: AtomicU64::new(already),
        abort: AtomicBool::new(false),
        failed: Mutex::new(None),
    };
    progress(already, total);
    let workers = connections.clamp(1, MOST_CONNECTIONS).min(chunks.max(1));
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers).map(|_| scope.spawn(|| worker(agent, source, &shared, cancel))).collect();
        let mut last = already;
        while !handles.iter().all(|handle| handle.is_finished()) {
            std::thread::sleep(PROGRESS_TICK);
            let now = shared.bytes.load(Ordering::Relaxed);
            if now != last {
                last = now;
                progress(now, total);
            }
        }
        for handle in handles {
            let _ = handle.join();
        }
    });
    progress(shared.bytes.load(Ordering::Relaxed), total);
    if cancel.load(Ordering::Relaxed) {
        return Err(Error::Cancelled);
    }
    if let Some(err) = shared.failed.lock().unwrap().take() {
        return Err(err);
    }
    Ok(())
}

/// One worker: the next chunk, until there is none, the file failed, or the user stopped it.
fn worker(agent: &ureq::Agent, source: &Source, shared: &Shared, cancel: &AtomicBool) {
    loop {
        if cancel.load(Ordering::Relaxed) || shared.abort.load(Ordering::Relaxed) {
            return;
        }
        let Some(index) = shared.pending.lock().unwrap().pop_front() else { return };
        match fetch_chunk(agent, source, shared, index, cancel) {
            Ok(()) => {
                let mut done = shared.done.lock().unwrap();
                done[index] = true;
                write_state(&shared.state_path, shared.total, &done);
            }
            Err(Error::Cancelled) => return,
            Err(err) => {
                shared.abort.store(true, Ordering::Relaxed);
                let mut failed = shared.failed.lock().unwrap();
                if failed.is_none() {
                    *failed = Some(err);
                }
                return;
            }
        }
    }
}

/// Chunk `index`, tried again from the last byte written when the connection fails.
fn fetch_chunk(agent: &ureq::Agent, source: &Source, shared: &Shared, index: usize, cancel: &AtomicBool) -> Result<(), Error> {
    let start = index as u64 * CHUNK_SIZE;
    let end = start + chunk_length(index, shared.total, CHUNK_SIZE) - 1;
    let mut written: u64 = 0;
    let mut attempts: u32 = 0;
    let mut buffer = vec![0u8; BUFFER_SIZE];
    loop {
        if cancel.load(Ordering::Relaxed) || shared.abort.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let before = written;
        match fetch_range(agent, source, start + written, end, shared, &mut written, &mut buffer, cancel) {
            Ok(()) => return Ok(()),
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            // The disk is the one thing trying again will not help. Everything else may pass next
            // time, as vget has it: a connection that drops, a 429 or a 5xx from a busy server, a
            // 404 from a CDN node that does not have the file yet (PikPak does that), a 200 where a
            // 206 was due.
            Err(err @ Error::Disk(_)) => return Err(err),
            Err(err) => {
                attempts = if written > before { 1 } else { attempts + 1 };
                if attempts >= ATTEMPTS {
                    return Err(err);
                }
                wait(backoff(attempts), cancel, &shared.abort)?;
            }
        }
    }
}

/// Bytes `from` to `end` of the file, written where they belong; `written` counts what arrived.
#[allow(clippy::too_many_arguments)]
fn fetch_range(
    agent: &ureq::Agent,
    source: &Source,
    from: u64,
    end: u64,
    shared: &Shared,
    written: &mut u64,
    buffer: &mut [u8],
    cancel: &AtomicBool,
) -> Result<(), Error> {
    let mut request = agent.get(source.url.as_str()).header("Range", format!("bytes={from}-{end}"));
    if let Some(authorization) = &source.authorization {
        request = request.header("Authorization", authorization.as_str());
    }
    let mut response = request.call()?;
    if response.status().as_u16() != 206 {
        return Err(Error::Protocol("the server ignored the range it said it would answer".into()));
    }
    let mut reader = response.body_mut().as_reader();
    let mut offset = from;
    loop {
        let remaining = (end + 1).saturating_sub(offset);
        if remaining == 0 {
            // One more read, to the body's end: only then does ureq keep the connection for the
            // next chunk instead of closing it, which would cost a new connection, its handshake
            // and its slow start for every 8 MB. A server that sends more than the range asked
            // for gets its connection closed instead; the file is not touched.
            let _ = reader.read(&mut buffer[..1]);
            return Ok(());
        }
        let want = usize::try_from(remaining).map_or(buffer.len(), |remaining| remaining.min(buffer.len()));
        let count = reader.read(&mut buffer[..want]).map_err(|err| Error::Connection(err.to_string()))?;
        if count == 0 {
            return Err(Error::Connection("the connection closed before the end of the range".into()));
        }
        // Never past the chunk's end: `want` saw to that.
        shared.file.write_all_at(&buffer[..count], offset).map_err(disk)?;
        offset += count as u64;
        *written += count as u64;
        shared.bytes.fetch_add(count as u64, Ordering::Relaxed);
        if cancel.load(Ordering::Relaxed) || shared.abort.load(Ordering::Relaxed) {
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
fn probe_with_retries(agent: &ureq::Agent, source: &Source, cancel: &AtomicBool) -> Result<Probe, Error> {
    let never = AtomicBool::new(false);
    let mut attempts: u32 = 0;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        match probe(agent, source) {
            Ok(probe) => return Ok(probe),
            Err(err) => {
                attempts += 1;
                if attempts >= ATTEMPTS {
                    return Err(err);
                }
                wait(backoff(attempts), cancel, &never)?;
            }
        }
    }
}

/// Sleep `pause`, unless the user stops the file or another worker fails meanwhile.
fn wait(pause: Duration, cancel: &AtomicBool, abort: &AtomicBool) -> Result<(), Error> {
    let until = Instant::now() + pause;
    while Instant::now() < until {
        if cancel.load(Ordering::Relaxed) || abort.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        std::thread::sleep(PROGRESS_TICK);
    }
    Ok(())
}

/// One connection, from the start: for a server that does not do ranges. Nothing to go on from
/// when it fails.
fn single(agent: &ureq::Agent, source: &Source, part: &Path, size: Option<u64>, cancel: &AtomicBool, progress: &dyn Fn(u64, u64)) -> Result<(), Error> {
    let mut request = agent.get(source.url.as_str());
    if let Some(authorization) = &source.authorization {
        request = request.header("Authorization", authorization.as_str());
    }
    let mut response = request.call()?;
    let total = response.body().content_length().or(size).unwrap_or(0);
    let mut file = File::create(part).map_err(disk)?;
    let mut reader = response.body_mut().as_reader();
    let mut buffer = vec![0u8; BUFFER_SIZE];
    let mut done: u64 = 0;
    let mut reported = Instant::now();
    progress(0, total);
    loop {
        let count = reader.read(&mut buffer).map_err(|err| Error::Connection(err.to_string()))?;
        if count == 0 {
            break;
        }
        file.write_all(&buffer[..count]).map_err(disk)?;
        done += count as u64;
        if reported.elapsed() >= PROGRESS_TICK {
            progress(done, total.max(done));
            reported = Instant::now();
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

fn write_state(path: &Path, total: u64, done: &[bool]) {
    let state = State { size: total, chunk_size: CHUNK_SIZE, chunks: done.len(), done: done.to_vec() };
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
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;

    use super::*;

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
        let agent = crate::downloads::webdav::agent(connections);
        download(&agent, &source(server), dest, connections, &AtomicBool::new(false), &|_, _| {})
    }

    #[test]
    fn the_probe_tells_the_size_and_whether_ranges_work() {
        let agent = crate::downloads::webdav::agent(2);
        let with = serve(pattern(1000), true, None);
        assert_eq!(probe(&agent, &source(&with)).unwrap(), Probe { size: Some(1000), ranges: true });
        let without = serve(pattern(1000), false, None);
        assert_eq!(probe(&agent, &source(&without)).unwrap(), Probe { size: Some(1000), ranges: false });
        let empty = serve(Vec::new(), true, None);
        assert_eq!(probe(&agent, &source(&empty)).unwrap(), Probe { size: Some(0), ranges: true });
        assert_eq!(content_range_total("bytes 0-1/12345"), Some(12345));
        assert_eq!(content_range_total("bytes 0-1/*"), None);
    }

    #[test]
    fn a_file_arrives_in_chunks_over_several_connections() {
        let content = pattern(2 * CHUNK_SIZE as usize + 12345);
        let server = serve(content.clone(), true, None);
        let scratch = Scratch::new("chunks");
        let dest = scratch.path("film.mkv");
        let agent = crate::downloads::webdav::agent(4);
        let seen = Mutex::new(Vec::new());
        let got = download(&agent, &source(&server), &dest, 4, &AtomicBool::new(false), &|done, total| seen.lock().unwrap().push((done, total))).expect("the file");
        assert_eq!(got, dest);
        assert_eq!(fs::read(&dest).unwrap(), content);
        assert!(!with_suffix(&dest, ".part").exists());
        assert!(!with_suffix(&dest, ".part.json").exists());
        let seen = seen.into_inner().unwrap();
        assert_eq!(seen.last(), Some(&(content.len() as u64, content.len() as u64)));
        assert!(seen.windows(2).all(|pair| pair[0].0 <= pair[1].0), "progress never goes back: {seen:?}");
        let mut requests = server.requests.lock().unwrap().clone();
        requests.sort();
        let last = content.len() - 1;
        assert_eq!(requests, vec!["bytes=0-1".to_string(), format!("bytes=0-{}", CHUNK_SIZE - 1), format!("bytes={}-{last}", 2 * CHUNK_SIZE), format!("bytes={}-{}", CHUNK_SIZE, 2 * CHUNK_SIZE - 1)]);
    }

    #[test]
    fn connections_are_kept_between_chunks() {
        // Twelve chunks over two workers take a handful of connections, each serving chunk after
        // chunk (the probe's, and a couple opened at the start); without the body read to its
        // end, every chunk would open one of its own, thirteen in all.
        let content = pattern(11 * CHUNK_SIZE as usize + 7);
        let server = serve(content.clone(), true, None);
        let scratch = Scratch::new("keepalive");
        let dest = scratch.path("film.mkv");
        fetch(&server, &dest, 2).expect("the file");
        assert_eq!(fs::read(&dest).unwrap(), content);
        assert_eq!(server.requests.lock().unwrap().len(), 13);
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
        write_state(&with_suffix(&dest, ".part.json"), content.len() as u64, &[true, false, false]);
        assert_eq!(kept(&dest), CHUNK_SIZE);
        let got = fetch(&server, &dest, 2).expect("the file");
        assert_eq!(got, dest);
        assert_eq!(fs::read(&dest).unwrap(), content);
        let requests = server.requests.lock().unwrap().clone();
        assert!(!requests.iter().any(|range| range == &format!("bytes=0-{}", CHUNK_SIZE - 1)), "the first chunk is not fetched again: {requests:?}");
        assert_eq!(requests.len(), 3);
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
        let agent = crate::downloads::webdav::agent(2);
        let right = Source { url: server.url.clone(), authorization: Some("Basic abc".into()) };
        assert_eq!(download(&agent, &right, &scratch.path("film.mkv"), 2, &AtomicBool::new(false), &|_, _| {}), Ok(scratch.path("film.mkv")));
        let gone = Source { url: "http://127.0.0.1:1/film.mkv".into(), authorization: None };
        assert!(matches!(download(&agent, &gone, &scratch.path("gone.mkv"), 2, &AtomicBool::new(false), &|_, _| {}), Err(Error::Connection(_))));
    }

    #[test]
    fn cancelled_before_the_first_byte_fetches_nothing() {
        let content = pattern(2 * CHUNK_SIZE as usize);
        let server = serve(content, true, None);
        let scratch = Scratch::new("cancel");
        let dest = scratch.path("film.mkv");
        let agent = crate::downloads::webdav::agent(2);
        let cancel = AtomicBool::new(true);
        assert_eq!(download(&agent, &source(&server), &dest, 2, &cancel, &|_, _| {}), Err(Error::Cancelled));
        assert!(!dest.exists());
        assert_eq!(kept(&dest), 0);
        assert!(server.requests.lock().unwrap().is_empty());
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
    fn the_last_chunk_is_shorter() {
        let total = 2 * CHUNK_SIZE + 123;
        assert_eq!(chunk_length(0, total, CHUNK_SIZE), CHUNK_SIZE);
        assert_eq!(chunk_length(1, total, CHUNK_SIZE), CHUNK_SIZE);
        assert_eq!(chunk_length(2, total, CHUNK_SIZE), 123);
        assert_eq!(chunk_length(3, total, CHUNK_SIZE), 0);
    }
}
