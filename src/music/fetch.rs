//! A song fetched over HTTP as it plays: its bytes go into a file of the cache, and the music mpv
//! reads them from there through a stream (src/player/stream.rs) as they arrive, waiting at the
//! edge; when mpv seeks past what has arrived, the fetch moves there, and comes back for what it
//! skipped. One thread per song. A file fetched whole stays in the cache for the next time, under
//! its own name; one on its way is ".part", each fetch's its own, and gone when the fetch is given
//! up or fails (the next one starts over).
//!
//! The music's mpv opens files only (docs/architecture.md): this is where the network is, the
//! app's own HTTP client, with the Keychain's roots.
//!
//! The server is the user's own, and still not believed about how much it sends: a song is never
//! written past the size the server said it has, one of no size said never past
//! MOST_WITHOUT_A_SIZE, and never into the last room of the disk (the downloads' margin,
//! src/downloads/transfer.rs). The files are written by name within the cache's folders, through
//! no symbolic link (the downloads' `Folder`): a link put into the cache under a song's name, or
//! in place of a folder, is not written through.

use std::io::Read as _;
use std::os::unix::fs::FileExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use crate::downloads::transfer;
use crate::player::stream::{self, Stream};

/// Bytes read from the server at a time, and written to the file.
const CHUNK: usize = 64 << 10;
/// The most a song may bring when the server does not say its size (a stream it transcodes as
/// it sends): fifteen hours at 320 kbit/s. A server that goes on past it is not sending a song.
const MOST_WITHOUT_A_SIZE: u64 = 2 << 30;
/// The room on the disk is asked about again every so many bytes written: well under the margin
/// that is kept free (`transfer::check_space`), so that most of the margin is always there.
const SPACE_ASKED_EVERY: u64 = 32 << 20;

/// Each fetch writes a file of its own: two of one song (the song queued twice) do not write
/// into each other's, and one given up removes only what is its.
static NEXT_PART: AtomicU64 = AtomicU64::new(1);

/// A song on its way, or there: the stream mpv reads it through, the URL it is offered under,
/// and the thread fetching it.
pub struct Fetch {
    /// "finchgram://n": what mpv plays.
    pub url: String,
    /// The file the song is written into while on its way.
    part: PathBuf,
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
    /// Fetch `http_url` into a ".part" beside `whole`, renamed to `whole` once it is all there;
    /// `root` is the cache's folder, which `whole` is under; `size` the song's size when known (0
    /// otherwise); `progress(from, to)` is told what has arrived, as parts of the file, from the
    /// fetching thread.
    pub fn start(agent: ureq::Agent, http_url: String, root: PathBuf, whole: PathBuf, size: u64, progress: impl Fn(f32, f32) + Send + Sync + 'static) -> Fetch {
        let part = transfer::with_suffix(&whole, &format!(".{}.part", NEXT_PART.fetch_add(1, Ordering::Relaxed)));
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
            let (stream, control, done, failed, part) = (stream.clone(), control.clone(), done.clone(), failed.clone(), part.clone());
            move || match fetch(&agent, &http_url, &root, &part, &whole, &stream, &control) {
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
        Fetch { url, part, control, done, failed }
    }

    /// The file the song is written into while on its way: the cache leaves it alone (mod.rs).
    pub fn part(&self) -> &Path {
        &self.part
    }

    /// The whole file is in the cache.
    pub fn is_done(&self) -> bool {
        self.done.load(Ordering::Acquire)
    }

    pub fn is_failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }

    /// Over: the stream is withdrawn, a read under way returns, the thread stops and removes
    /// what it had written; the song is fetched from its start next time.
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

/// The fetch itself, on its thread: true once the file is whole, false when cancelled. The file
/// it writes is its own, under `root` through no link (`transfer::open_own`): whatever is under
/// its name goes first (a file left by a FinchGram that quit while it fetched, or a link put
/// there), and so does the file itself when the fetch is given up or fails.
fn fetch(agent: &ureq::Agent, http_url: &str, root: &Path, part: &Path, whole: &Path, stream: &Stream, control: &Control) -> Result<bool, String> {
    transfer::remove_own(root, part);
    let file = transfer::open_own(root, part, true).map_err(|err| format!("cannot write {}: {err}", part.display()))?;
    let outcome = bring(agent, http_url, root, part, whole, &file, stream, control);
    if !matches!(outcome, Ok(true)) {
        transfer::remove_own(root, part);
    }
    outcome
}

/// The song's bytes into `file` (which is at `part`), range by range, until all of it is there
/// and the file is renamed `whole`.
#[allow(clippy::too_many_arguments)]
fn bring(agent: &ureq::Agent, http_url: &str, root: &Path, part: &Path, whole: &Path, file: &std::fs::File, stream: &Stream, control: &Control) -> Result<bool, String> {
    // What is on the disk, as (from, to) ranges in order, merged.
    let mut ranges: Vec<(u64, u64)> = Vec::new();
    let mut total: Option<u64> = None;
    let mut from: u64 = 0;
    // The room on the disk is asked about before anything is written, when the song's size is
    // known, and again every so many bytes written: a server that sends without end stops at the
    // margin, not at a full disk.
    transfer::check_space(part, 0).map_err(|err| err.to_string())?;
    let mut unchecked: u64 = 0;
    let held = |ranges: &[(u64, u64)]| ranges.iter().map(|(from, to)| to - from).sum::<u64>();
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
        let had = held(&ranges);
        if start != from {
            // The server does not do ranges (200 from the start): everything comes again.
            ranges.clear();
        }
        if let Some(size) = size
            && total != Some(size)
        {
            total = Some(size);
            transfer::check_space(part, size.saturating_sub(held(&ranges))).map_err(|err| err.to_string())?;
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
            let to = position + read as u64;
            within(to, total)?;
            unchecked += read as u64;
            if unchecked >= SPACE_ASKED_EVERY {
                unchecked = 0;
                transfer::check_space(part, 0).map_err(|err| err.to_string())?;
            }
            file.write_all_at(&buffer[..read], position).map_err(|err| format!("cannot write {}: {err}", part.display()))?;
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
            transfer::rename_own(root, part, whole).map_err(|err| format!("cannot keep {}: {err}", whole.display()))?;
            stream.arrived(Some(whole.to_path_buf()), 0, whole_size, true, whole_size);
            return Ok(true);
        }
        if status == 200 && total.is_none() {
            // Everything came and still is not all of it: the size was wrong somewhere.
            return Err("the file came short".into());
        }
        if held(&ranges) <= had {
            // An answer that brought nothing the file did not have: asking again would go on
            // for as long as the server answers that way.
            return Err("the server sent nothing new".into());
        }
        // A gap to fill: the first byte not on the disk.
        from = first_gap(&ranges, whole_size).ok_or("the file came short")?;
    }
}

/// Whether a song that has come as far as byte `to` is still within what it may bring: the size
/// the server said it has (`total`), or MOST_WITHOUT_A_SIZE when it said none.
fn within(to: u64, total: Option<u64>) -> Result<(), String> {
    match total {
        Some(total) if to > total => Err(format!("the server sent more than the {total} bytes it said the song has")),
        None if to > MOST_WITHOUT_A_SIZE => Err(format!("the server said no size and sent more than {} MB", MOST_WITHOUT_A_SIZE >> 20)),
        _ => Ok(()),
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
        let whole = folder.join("source").join("song.mp3");
        let shown = Arc::new(Mutex::new(Vec::new()));
        let fetch = Fetch::start(ureq::Agent::new_with_defaults(), address, folder.clone(), whole.clone(), content.len() as u64, {
            let shown = shown.clone();
            move |from, to| shown.lock().unwrap().push((from, to))
        });
        assert!(fetch.url.starts_with("finchgram://"));
        // mpv seeks far ahead: the fetch goes there, then comes back for the start.
        fetch.control.ask(600_000);
        assert!(wait_until(|| fetch.is_done()), "fetched within ten seconds");
        assert!(!fetch.is_failed());
        assert_eq!(std::fs::read(&whole).expect("the whole file"), content);
        // The file it was written into is beside it while on its way, and is the song itself now.
        assert!(fetch.part().starts_with(folder.join("source")) && fetch.part().extension().is_some_and(|extension| extension == "part"));
        assert!(!fetch.part().exists());
        let shown = shown.lock().unwrap();
        assert_eq!(shown.last(), Some(&(0.0, 0.0)));
        assert!(shown.iter().any(|(from, _)| (*from - 0.6666).abs() < 0.01), "a window from the seek among {:?}", &shown[..shown.len().min(12)]);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_server_without_ranges_gives_the_whole_file_and_a_missing_one_fails_the_stream() {
        let content = pattern(100_000);
        let folder = scratch("plain");
        let fetch = Fetch::start(ureq::Agent::new_with_defaults(), serve(content.clone(), false, true), folder.clone(), folder.join("a.mp3"), 0, |_, _| {});
        assert!(wait_until(|| fetch.is_done()));
        assert_eq!(std::fs::read(folder.join("a.mp3")).expect("the file"), content);
        let missing = Fetch::start(ureq::Agent::new_with_defaults(), serve(Vec::new(), true, false), folder.clone(), folder.join("b.mp3"), 0, |_, _| {});
        assert!(wait_until(|| missing.is_failed()));
        assert!(!folder.join("b.mp3").exists());
        // What a fetch that failed had written is gone with it: nothing but the song is left.
        assert!(wait_until(|| !missing.part().exists()));
        assert_eq!(std::fs::read_dir(&folder).expect("the folder").flatten().map(|entry| entry.file_name()).collect::<Vec<_>>(), ["a.mp3"]);
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// A server that answers every request with `head` and then `body`.
    fn answer(head: String, body: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
        let address = format!("http://{}/song.mp3", listener.local_addr().expect("the address"));
        std::thread::spawn(move || {
            for connection in listener.incoming() {
                let Ok(mut connection) = connection else { break };
                let mut request = Vec::new();
                let mut byte = [0u8; 1];
                while !request.ends_with(b"\r\n\r\n") && connection.read(&mut byte).map(|read| read == 1).unwrap_or(false) {
                    request.push(byte[0]);
                }
                let _ = connection.write_all(head.as_bytes());
                let _ = connection.write_all(&body);
            }
        });
        address
    }

    #[test]
    fn a_song_is_not_written_past_the_size_the_server_said() {
        // "Ten bytes of the song", and a hundred thousand sent.
        let head = "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-9/10\r\nContent-Length: 100000\r\nConnection: close\r\n\r\n".to_string();
        let folder = scratch("too-much");
        let fetch = Fetch::start(ureq::Agent::new_with_defaults(), answer(head, pattern(100_000)), folder.clone(), folder.join("song.mp3"), 0, |_, _| {});
        assert!(wait_until(|| fetch.is_failed()));
        assert!(wait_until(|| !fetch.part().exists()));
        assert!(!folder.join("song.mp3").exists());
        assert_eq!(within(10, Some(10)), Ok(()));
        assert!(within(11, Some(10)).is_err());
        // No size said (a stream transcoded as it is sent): a ceiling of its own.
        assert_eq!(within(MOST_WITHOUT_A_SIZE, None), Ok(()));
        assert!(within(MOST_WITHOUT_A_SIZE + 1, None).is_err());
        // A size said is the song's, however large.
        assert_eq!(within(MOST_WITHOUT_A_SIZE + 1, Some(MOST_WITHOUT_A_SIZE * 4)), Ok(()));
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_server_that_keeps_sending_the_same_bytes_is_given_up() {
        // The first ten bytes of a hundred, whatever is asked for: the fetch would ask for the
        // rest again and again.
        let head = "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-9/100\r\nContent-Length: 10\r\nConnection: close\r\n\r\n".to_string();
        let folder = scratch("same-bytes");
        let fetch = Fetch::start(ureq::Agent::new_with_defaults(), answer(head, pattern(10)), folder.clone(), folder.join("song.mp3"), 0, |_, _| {});
        assert!(wait_until(|| fetch.is_failed()));
        assert!(wait_until(|| !fetch.part().exists()));
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_link_in_the_cache_is_not_written_through() {
        let content = pattern(50_000);
        let folder = scratch("links");
        let (root, outside) = (folder.join("cache"), folder.join("outside"));
        std::fs::create_dir_all(root.join("source")).expect("folders");
        std::fs::create_dir_all(&outside).expect("folders");
        let fetched = |part: &Path, whole: &Path| {
            let stream = Stream::new(0, |_| {}, |_, _| {});
            fetch(&ureq::Agent::new_with_defaults(), &serve(content.clone(), true, true), &root, part, whole, &stream, &Control::new())
        };
        // A link under the very name the song is written to while on its way: the link goes,
        // what it pointed at stays as it was, and the song is fetched into a file of its own.
        let victim = outside.join("notes.txt");
        std::fs::write(&victim, b"the user's own words").expect("a file");
        let (part, whole) = (root.join("source").join("song.mp3.1.part"), root.join("source").join("song.mp3"));
        std::os::unix::fs::symlink(&victim, &part).expect("a link");
        assert_eq!(fetched(&part, &whole), Ok(true));
        assert_eq!(std::fs::read(&victim).expect("the file"), b"the user's own words");
        assert_eq!(std::fs::read(&whole).expect("the song"), content);
        assert!(std::fs::symlink_metadata(&part).is_err());
        // A link in place of the source's folder: nothing is written where it leads.
        std::os::unix::fs::symlink(&outside, root.join("linked")).expect("a link");
        let (part, whole) = (root.join("linked").join("song.mp3.2.part"), root.join("linked").join("song.mp3"));
        assert!(fetched(&part, &whole).is_err());
        assert_eq!(std::fs::read_dir(&outside).expect("the folder").count(), 1);
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
