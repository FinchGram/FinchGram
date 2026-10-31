//! A song fetched over HTTP as it plays: its bytes go into a file of the cache, and the music mpv
//! reads them from there through a stream (src/player/stream.rs) as they arrive, waiting at the
//! edge; when mpv seeks past what has arrived, the fetch moves there, and comes back for what it
//! skipped. One thread per song. A file fetched whole stays in the cache for the next time, under
//! its own name; one on its way is ".part".
//!
//! The vendored FFmpeg has no network code (docs/architecture.md): this is where the network is,
//! the app's own HTTP client, with the Keychain's roots.

use std::fs::OpenOptions;
use std::io::Read as _;
use std::os::unix::fs::FileExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use crate::player::stream::{self, Stream};

/// Bytes read from the server at a time, and written to the file.
const CHUNK: usize = 64 << 10;

/// A song on its way, or there: the stream mpv reads it through, the URL it is offered under,
/// and the thread fetching it.
pub struct Fetch {
    /// "finchgram://n": what mpv plays.
    pub url: String,
    control: Arc<Control>,
    done: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
}

/// What the stream and the owner ask of the fetching thread.
struct Control {
    wanted: Mutex<Wanted>,
    changed: Condvar,
}

struct Wanted {
    /// The offset mpv reads at and the fetch is not bringing.
    asked: Option<u64>,
    cancelled: bool,
}

impl Control {
    fn new() -> Control {
        Control { wanted: Mutex::new(Wanted { asked: None, cancelled: false }), changed: Condvar::new() }
    }

    fn ask(&self, offset: u64) {
        self.wanted.lock().unwrap().asked = Some(offset);
        self.changed.notify_all();
    }

    fn cancel(&self) {
        self.wanted.lock().unwrap().cancelled = true;
        self.changed.notify_all();
    }

    fn take_asked(&self) -> Option<u64> {
        self.wanted.lock().unwrap().asked.take()
    }

    fn is_cancelled(&self) -> bool {
        self.wanted.lock().unwrap().cancelled
    }
}

impl Fetch {
    /// Fetch `http_url` into `part`, renamed to `whole` once it is all there; `size` the song's
    /// size when known (0 otherwise); `progress(from, to)` is told what has arrived, as parts of
    /// the file, from the fetching thread.
    pub fn start(agent: ureq::Agent, http_url: String, part: PathBuf, whole: PathBuf, size: u64, progress: impl Fn(f32, f32) + Send + Sync + 'static) -> Fetch {
        let control = Arc::new(Control::new());
        let stream = Stream::new(
            size,
            {
                let control = control.clone();
                move |offset| control.ask(offset)
            },
            progress,
        );
        let url = stream::offer_music(stream.clone());
        let done = Arc::new(AtomicBool::new(false));
        let failed = Arc::new(AtomicBool::new(false));
        let started = std::thread::Builder::new().name("music-fetch".into()).spawn({
            let (stream, control, done, failed) = (stream.clone(), control.clone(), done.clone(), failed.clone());
            move || match fetch(&agent, &http_url, &part, &whole, &stream, &control) {
                Ok(true) => done.store(true, Ordering::Release),
                Ok(false) => {}
                Err(err) => {
                    eprintln!("music: cannot fetch {}: {err}", whole.display());
                    failed.store(true, Ordering::Release);
                    stream.failed();
                }
            }
        });
        if let Err(err) = started {
            eprintln!("music: cannot start a fetching thread: {err}");
            failed.store(true, Ordering::Release);
            stream.failed();
        }
        Fetch { url, control, done, failed }
    }

    /// The whole file is in the cache.
    pub fn is_done(&self) -> bool {
        self.done.load(Ordering::Acquire)
    }

    pub fn is_failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }

    /// Over: the stream is withdrawn, a read under way returns, the thread stops; a partial file
    /// stays and is fetched again next time.
    pub fn cancel(&self) {
        self.control.cancel();
        stream::withdraw_music(&self.url);
    }
}

impl Drop for Fetch {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// The fetch itself, on its thread: true once the file is whole, false when cancelled.
fn fetch(agent: &ureq::Agent, http_url: &str, part: &Path, whole: &Path, stream: &Stream, control: &Control) -> Result<bool, String> {
    if let Some(folder) = part.parent() {
        std::fs::create_dir_all(folder).map_err(|err| format!("cannot make {}: {err}", folder.display()))?;
    }
    let file = OpenOptions::new().create(true).write(true).truncate(true).open(part).map_err(|err| format!("cannot write {}: {err}", part.display()))?;
    // What is on the disk, as (from, to) ranges in order, merged.
    let mut ranges: Vec<(u64, u64)> = Vec::new();
    let mut total: Option<u64> = None;
    let mut from: u64 = 0;
    loop {
        if control.is_cancelled() {
            return Ok(false);
        }
        // From `from` to the end, or up to the next range already there.
        let until = ranges.iter().find(|(start, _)| *start > from).map(|(start, _)| *start);
        let range = match until {
            Some(until) => format!("bytes={from}-{}", until - 1),
            None => format!("bytes={from}-"),
        };
        let mut response = agent.get(http_url).header("Range", &range).call().map_err(|err| err.to_string())?;
        let status = response.status().as_u16();
        let header = |name: &str| response.headers().get(name).and_then(|value| value.to_str().ok()).map(str::to_string);
        let (start, size) = match status {
            206 => content_range(&header("content-range").unwrap_or_default()).ok_or("a 206 without a Content-Range")?,
            200 => (0, header("content-length").and_then(|length| length.parse::<u64>().ok())),
            _ => return Err(format!("HTTP {status}")),
        };
        if start != from {
            // The server does not do ranges (200 from the start): everything comes again.
            ranges.clear();
        }
        if let Some(size) = size {
            total = Some(size);
        }
        let mut position = start;
        let mut buffer = vec![0u8; CHUNK];
        let mut moved: Option<u64> = None;
        let mut reader = response.body_mut().as_reader();
        loop {
            if control.is_cancelled() {
                return Ok(false);
            }
            if let Some(asked) = control.take_asked() {
                if let Some(window) = window_of(&ranges, asked) {
                    tell(stream, part, window, total);
                } else if asked >= position + (CHUNK as u64) * 8 || asked < position {
                    // mpv wants bytes far from here: the fetch moves there, and comes back later.
                    moved = Some(asked);
                    break;
                }
            }
            let read = reader.read(&mut buffer).map_err(|err| err.to_string())?;
            if read == 0 {
                break;
            }
            file.write_all_at(&buffer[..read], position).map_err(|err| format!("cannot write {}: {err}", part.display()))?;
            let to = position + read as u64;
            add(&mut ranges, position, to);
            position = to;
            if let Some(window) = window_of(&ranges, to - 1) {
                tell(stream, part, window, total);
            }
        }
        drop(reader);
        if let Some(asked) = moved {
            from = asked;
            continue;
        }
        let whole_size = match total {
            Some(total) => total,
            // No size from the server (a transcoded stream): what came is all of it.
            None => ranges.last().map_or(0, |(_, to)| *to),
        };
        if ranges.len() == 1 && ranges[0] == (0, whole_size) {
            std::fs::rename(part, whole).map_err(|err| format!("cannot keep {}: {err}", whole.display()))?;
            stream.arrived(Some(whole.to_path_buf()), 0, whole_size, true, whole_size);
            return Ok(true);
        }
        if status == 200 && total.is_none() {
            // Everything came and still is not all of it: the size was wrong somewhere.
            return Err("the file came short".into());
        }
        // A gap to fill: the first byte not on the disk.
        from = first_gap(&ranges, whole_size).ok_or("the file came short")?;
    }
}

/// Tell the stream what is on the disk around where mpv reads.
fn tell(stream: &Stream, part: &Path, (from, to): (u64, u64), total: Option<u64>) {
    stream.arrived(Some(part.to_path_buf()), from, to - from, false, total.unwrap_or(0));
}

/// "bytes 0-1023/5365035": the first byte and the whole size (none for "*").
fn content_range(header: &str) -> Option<(u64, Option<u64>)> {
    let rest = header.trim().strip_prefix("bytes ")?;
    let (range, size) = rest.split_once('/')?;
    let start = range.split('-').next()?.trim().parse::<u64>().ok()?;
    let total = if size.trim() == "*" { None } else { Some(size.trim().parse::<u64>().ok()?) };
    Some((start, total))
}

/// `from … to` added to the ranges, merged with its neighbours.
fn add(ranges: &mut Vec<(u64, u64)>, from: u64, to: u64) {
    if to <= from {
        return;
    }
    let mut merged = (from, to);
    ranges.retain(|range| {
        if range.1 < merged.0 || range.0 > merged.1 {
            return true;
        }
        merged = (merged.0.min(range.0), merged.1.max(range.1));
        false
    });
    let at = ranges.iter().position(|range| range.0 > merged.0).unwrap_or(ranges.len());
    ranges.insert(at, merged);
}

/// The range `at` falls in, if any.
fn window_of(ranges: &[(u64, u64)], at: u64) -> Option<(u64, u64)> {
    ranges.iter().copied().find(|(from, to)| *from <= at && at < *to)
}

/// The first byte under `size` that no range holds.
fn first_gap(ranges: &[(u64, u64)], size: u64) -> Option<u64> {
    let mut at = 0;
    for (from, to) in ranges {
        if *from > at {
            return Some(at);
        }
        at = at.max(*to);
    }
    (at < size).then_some(at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    /// A server with one file, doing ranges (206) or not (200 for everything), or refusing (404);
    /// the body goes out in pieces, slowly enough for a seek to come while it is on its way.
    fn serve(content: Vec<u8>, ranges: bool, found: bool) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
        let address = format!("http://{}/song.mp3", listener.local_addr().expect("the address"));
        std::thread::spawn(move || {
            for connection in listener.incoming() {
                let Ok(mut connection) = connection else { break };
                let content = content.clone();
                std::thread::spawn(move || {
                    let mut request = Vec::new();
                    let mut byte = [0u8; 1];
                    while !request.ends_with(b"\r\n\r\n") && connection.read(&mut byte).map(|read| read == 1).unwrap_or(false) {
                        request.push(byte[0]);
                    }
                    let request = String::from_utf8_lossy(&request).to_string();
                    // ureq writes header names in lower case.
                    let range = request.lines().find_map(|line| line.to_ascii_lowercase().strip_prefix("range: bytes=").map(str::to_string));
                    let response = if !found {
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
                    } else if let (true, Some(range)) = (ranges, range) {
                        let (from, to) = range.split_once('-').expect("a range");
                        let from: usize = from.parse().expect("a number");
                        let to: usize = if to.is_empty() { content.len() - 1 } else { to.parse().expect("a number") };
                        let body = &content[from..=to];
                        let mut response = format!("HTTP/1.1 206 Partial Content\r\nContent-Type: audio/mpeg\r\nContent-Length: {}\r\nContent-Range: bytes {from}-{to}/{}\r\nConnection: close\r\n\r\n", body.len(), content.len()).into_bytes();
                        response.extend_from_slice(body);
                        response
                    } else {
                        let mut response = format!("HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", content.len()).into_bytes();
                        response.extend_from_slice(&content);
                        response
                    };
                    for piece in response.chunks(16 << 10) {
                        if connection.write_all(piece).is_err() {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(2));
                    }
                });
            }
        });
        address
    }

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("finchgram-fetch-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        folder
    }

    fn pattern(length: usize) -> Vec<u8> {
        (0..length).map(|at| (at % 251) as u8).collect()
    }

    fn wait_until(what: impl Fn() -> bool) -> bool {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(10) {
            if what() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn a_song_is_fetched_whole_into_the_cache_and_a_seek_moves_the_fetch() {
        let content = pattern(900_000);
        let address = serve(content.clone(), true, true);
        let folder = scratch("ranges");
        let (part, whole) = (folder.join("song.part"), folder.join("song.mp3"));
        let shown = Arc::new(Mutex::new(Vec::new()));
        let fetch = Fetch::start(ureq::Agent::new_with_defaults(), address, part.clone(), whole.clone(), content.len() as u64, {
            let shown = shown.clone();
            move |from, to| shown.lock().unwrap().push((from, to))
        });
        assert!(fetch.url.starts_with("finchgram://"));
        // mpv seeks far ahead: the fetch goes there, then comes back for the start.
        fetch.control.ask(600_000);
        assert!(wait_until(|| fetch.is_done()), "fetched within ten seconds");
        assert!(!fetch.is_failed());
        assert_eq!(std::fs::read(&whole).expect("the whole file"), content);
        assert!(!part.exists());
        let shown = shown.lock().unwrap();
        assert_eq!(shown.last(), Some(&(0.0, 0.0)));
        assert!(shown.iter().any(|(from, _)| (*from - 0.6666).abs() < 0.01), "a window from the seek among {:?}", &shown[..shown.len().min(12)]);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_server_without_ranges_gives_the_whole_file_and_a_missing_one_fails_the_stream() {
        let content = pattern(100_000);
        let folder = scratch("plain");
        let fetch = Fetch::start(ureq::Agent::new_with_defaults(), serve(content.clone(), false, true), folder.join("a.part"), folder.join("a.mp3"), 0, |_, _| {});
        assert!(wait_until(|| fetch.is_done()));
        assert_eq!(std::fs::read(folder.join("a.mp3")).expect("the file"), content);
        let missing = Fetch::start(ureq::Agent::new_with_defaults(), serve(Vec::new(), true, false), folder.join("b.part"), folder.join("b.mp3"), 0, |_, _| {});
        assert!(wait_until(|| missing.is_failed()));
        assert!(!folder.join("b.mp3").exists());
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn ranges_merge_and_gaps_are_found() {
        let mut ranges = Vec::new();
        add(&mut ranges, 0, 10);
        add(&mut ranges, 20, 30);
        add(&mut ranges, 10, 15);
        assert_eq!(ranges, vec![(0, 15), (20, 30)]);
        assert_eq!(first_gap(&ranges, 30), Some(15));
        add(&mut ranges, 15, 20);
        assert_eq!(ranges, vec![(0, 30)]);
        assert_eq!(first_gap(&ranges, 30), None);
        assert_eq!(first_gap(&ranges, 40), Some(30));
        assert_eq!(window_of(&[(5, 10)], 7), Some((5, 10)));
        assert_eq!(window_of(&[(5, 10)], 10), None);
        assert_eq!(content_range("bytes 0-1023/5365035"), Some((0, Some(5365035))));
        assert_eq!(content_range("bytes 100-199/*"), Some((100, None)));
        assert_eq!(content_range("items 1-2/3"), None);
    }
}
