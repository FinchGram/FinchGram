//! The streams' thumbnails: fetched from their sources on a worker of their own, a few at a time,
//! into ~/Documents/FinchGram/live_thumbnails, then decoded off the UI thread (images.rs) for the
//! cards. A live thumbnail changes, so a file older than a few minutes is fetched again; the folder
//! keeps a day's worth.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Read as _;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, SystemTime};

use sha2::Digest as _;

/// A file younger than this is shown as it is.
const FRESH_FOR: Duration = Duration::from_secs(5 * 60);
/// Files older than this go at the start.
const KEPT_FOR: Duration = Duration::from_secs(24 * 60 * 60);
/// A thumbnail larger than this is not one.
const LARGEST: u64 = 4 * 1024 * 1024;
/// Thumbnails fetched at once.
const AT_ONCE: usize = 3;

struct Job {
    id: String,
    url: String,
    path: PathBuf,
}

thread_local! {
    static WORKER: RefCell<Option<Sender<Job>>> = const { RefCell::new(None) };
    /// The decoded ones, by address, for the cards shown again.
    static DECODED: RefCell<HashMap<String, (SystemTime, slint::Image)>> = RefCell::new(HashMap::new());
}

pub fn folder() -> PathBuf {
    dirs::document_dir().or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from(".")).join("FinchGram").join("live_thumbnails")
}

pub fn start() {
    let folder = folder();
    let _ = std::fs::create_dir_all(&folder);
    if let Ok(entries) = std::fs::read_dir(&folder) {
        for entry in entries.flatten() {
            let old = entry.metadata().and_then(|meta| meta.modified()).ok().and_then(|at| at.elapsed().ok()).is_none_or(|age| age > KEPT_FOR);
            if old {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    let (sender, jobs) = mpsc::channel::<Job>();
    let jobs = std::sync::Arc::new(std::sync::Mutex::new(jobs));
    for lane in 0..AT_ONCE {
        let jobs = jobs.clone();
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .user_agent(format!("FinchGram/{}", crate::update::CURRENT_VERSION))
            .timeout_connect(Some(Duration::from_secs(8)))
            .timeout_global(Some(Duration::from_secs(20)))
            .http_status_as_error(false)
            .build()
            .into();
        let _ = std::thread::Builder::new().name(format!("live thumbnails {lane}")).spawn(move || {
            loop {
                let job = match jobs.lock() {
                    Ok(jobs) => jobs.recv(),
                    Err(_) => return,
                };
                let Ok(job) = job else { return };
                let fetched = fetch_to(&agent, &job.url, &job.path);
                if let Err(err) = &fetched {
                    eprintln!("live: thumbnail {}: {err}", job.url);
                }
                let (id, url, path) = (job.id, job.url, job.path);
                let ok = fetched.is_ok();
                let _ = slint::invoke_from_event_loop(move || fetched_came(id, url, path, ok));
            }
        });
    }
    WORKER.with(|kept| *kept.borrow_mut() = Some(sender));
}

/// The thumbnail at `url` for the cards of channel `id`.
pub fn fetch(id: &str, url: &str) {
    let fresh = DECODED.with(|decoded| decoded.borrow().get(url).filter(|(at, _)| at.elapsed().is_ok_and(|age| age < FRESH_FOR)).map(|(_, image)| image.clone()));
    if let Some(image) = fresh {
        super::thumbnail_came(id, Some(image));
        return;
    }
    let path = folder().join(name_of(url));
    let on_disk = std::fs::metadata(&path).and_then(|meta| meta.modified()).ok().and_then(|at| at.elapsed().ok()).is_some_and(|age| age < FRESH_FOR);
    if on_disk {
        decode(id.to_string(), url.to_string(), path);
        return;
    }
    WORKER.with(|worker| {
        if let Some(worker) = worker.borrow().as_ref() {
            let _ = worker.send(Job { id: id.to_string(), url: url.to_string(), path });
        }
    });
}

fn name_of(url: &str) -> String {
    let digest = sha2::Sha256::digest(url.as_bytes());
    digest.iter().take(12).map(|byte| format!("{byte:02x}")).collect::<String>() + ".img"
}

fn fetch_to(agent: &ureq::Agent, url: &str, path: &std::path::Path) -> Result<(), String> {
    let mut response = agent.get(url).header("Accept", "image/*").call().map_err(|err| err.to_string())?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(format!("HTTP {status}"));
    }
    let mut bytes = Vec::new();
    response.body_mut().as_reader().take(LARGEST + 1).read_to_end(&mut bytes).map_err(|err| err.to_string())?;
    if bytes.len() as u64 > LARGEST || bytes.is_empty() {
        return Err("not a thumbnail".to_string());
    }
    let partial = path.with_extension("part");
    std::fs::write(&partial, &bytes).map_err(|err| err.to_string())?;
    std::fs::rename(&partial, path).map_err(|err| err.to_string())
}

fn fetched_came(id: String, url: String, path: PathBuf, ok: bool) {
    if ok {
        decode(id, url, path);
    } else {
        super::thumbnail_came(&id, None);
    }
}

fn decode(id: String, url: String, path: PathBuf) {
    crate::images::load(path, move |image| {
        if let Some(image) = &image {
            DECODED.with(|decoded| {
                let mut decoded = decoded.borrow_mut();
                if decoded.len() > 300 {
                    decoded.clear();
                }
                decoded.insert(url.clone(), (SystemTime::now(), image.clone()));
            });
        }
        super::thumbnail_came(&id, image);
    });
}
