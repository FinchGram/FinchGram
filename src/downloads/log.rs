//! A log of what the engine does, line by line, for a user whose server is slow and for the
//! maintainer: which protocol a server speaks, how long each request takes to answer and to
//! deliver, what was tried again and why. It is `downloads.log` next to settings.toml
//! (`~/Library/Application Support/FinchGram/` on macOS), started afresh once it passes 8 MB.
//! Addresses are logged without the account or the query that may be in them (`shown`),
//! passwords never.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

const MOST_BYTES: u64 = 8 * 1024 * 1024;

static LOG: OnceLock<Option<Mutex<File>>> = OnceLock::new();

/// Where the log is.
pub fn path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("FinchGram").join("downloads.log"))
}

fn open() -> Option<Mutex<File>> {
    // The tests' traffic is nobody's business.
    if cfg!(test) {
        return None;
    }
    let path = path()?;
    let _ = std::fs::create_dir_all(path.parent()?);
    let afresh = std::fs::metadata(&path).map(|meta| meta.len() > MOST_BYTES).unwrap_or(false);
    OpenOptions::new().create(true).write(true).append(!afresh).truncate(afresh).open(&path).ok().map(Mutex::new)
}

/// A line, with the time; nothing happens when the log cannot be written. The benchmark test
/// (transfer.rs, FINCHGRAM_BENCH_URL) sees the lines on stderr instead.
pub fn line(words: impl AsRef<str>) {
    if cfg!(test) && std::env::var_os("FINCHGRAM_BENCH_URL").is_some() {
        eprintln!("{} {}", chrono::Local::now().format("%H:%M:%S%.3f"), words.as_ref());
    }
    if let Some(file) = LOG.get_or_init(open)
        && let Ok(mut file) = file.lock()
    {
        let _ = writeln!(file, "{} {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"), words.as_ref());
    }
}

/// An address as the log shows it: the scheme, host and path, without an account that may be in
/// it ("user:pass@") and without the query (a CDN's signed link carries its token there) or the
/// fragment.
pub fn shown(url: &str) -> String {
    let (scheme, rest) = match url.split_once("://") {
        Some((scheme, rest)) => (format!("{scheme}://"), rest),
        None => (String::new(), url),
    };
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    let (authority, path) = match rest.find('/') {
        Some(at) => rest.split_at(at),
        None => (rest, ""),
    };
    let host = authority.rsplit('@').next().unwrap_or(authority);
    format!("{scheme}{host}{path}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_shown_without_its_account_query_or_fragment() {
        assert_eq!(shown("https://me:secret@nas.home:5006/dav/Movies/a%20b.mkv?sign=abc#x"), "https://nas.home:5006/dav/Movies/a%20b.mkv");
        assert_eq!(shown("https://dl-z01a-0026.mypikpak.net/download/?fid=1&sign=xyz"), "https://dl-z01a-0026.mypikpak.net/download/");
        assert_eq!(shown("http://nas.home/dav/"), "http://nas.home/dav/");
        assert_eq!(shown("http://nas.home"), "http://nas.home");
        assert_eq!(shown("/dav/x?y=1"), "/dav/x");
    }
}
