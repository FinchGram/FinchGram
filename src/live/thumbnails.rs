//! The streams' thumbnails: fetched from their sources on a worker of their own, a few at a time,
//! into ~/Documents/FinchGram/live_thumbnails, then decoded off the UI thread (images.rs) for the
//! cards. A live thumbnail changes, so a file older than a few minutes is fetched again; the folder
//! keeps a day's worth.
//!
//! A thumbnail's address is a stranger's word (a stream's own, passed on by the directory), and it
//! is fetched without the user asking, for every card shown. So the client connects to addresses
//! of the public internet only (`PublicOnly`): an address that names this Mac, the home network
//! or another private one, at once or after a redirect, would otherwise have FinchGram send a
//! request there for whoever listed the stream.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Read as _;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, SystemTime};

use sha2::Digest as _;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};

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
        let agent = agent();
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

/// The thumbnails' HTTP client: as any of the app's, but for where it connects (`PublicOnly`).
fn agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .user_agent(format!("FinchGram/{}", crate::update::CURRENT_VERSION))
        .timeout_connect(Some(Duration::from_secs(8)))
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .build();
    ureq::Agent::with_parts(config, DefaultConnector::default(), PublicOnly::default())
}

/// Where the thumbnails' client may connect: the system's answer for a host, less every address
/// that is not one of the public internet (`is_public`). The client connects to the addresses
/// given here and no others, and asks again for each redirect, so a host is held to this at every
/// step and by the very addresses used, whatever its name says the next time it is looked up.
/// Through a proxy the user set for the app (the environment's), the connection is to the proxy,
/// theirs wherever it is, and the proxy looks the host up itself.
#[derive(Debug, Default)]
struct PublicOnly(DefaultResolver);

impl Resolver for PublicOnly {
    fn resolve(&self, uri: &ureq::http::Uri, config: &ureq::config::Config, timeout: NextTimeout) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let found = self.0.resolve(uri, config, timeout)?;
        if config.proxy().is_some_and(|proxy| proxy.uri() == uri) {
            return Ok(found);
        }
        let mut public = self.empty();
        for address in found.iter().filter(|address| is_public(address.ip())) {
            public.push(*address);
        }
        if public.is_empty() {
            return Err(ureq::Error::Io(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "not an address of the public internet")));
        }
        Ok(public)
    }
}

/// Whether `address` is one of the public internet: not this Mac (loopback), not a private or
/// shared network (10/8, 172.16/12, 192.168/16, 100.64/10, fc00::/7), not link-local (169.254/16,
/// where a cloud's metadata lives; fe80::/10), not unspecified, multicast, broadcast, reserved or
/// for documentation. An IPv6 address that carries an IPv4 one (mapped, compatible, NAT64, 6to4)
/// is what that one is.
fn is_public(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_public_v4(address),
        IpAddr::V6(address) => is_public_v6(address),
    }
}

fn is_public_v4(address: Ipv4Addr) -> bool {
    let [first, second, third, _] = address.octets();
    let special = address.is_unspecified()
        || address.is_loopback()
        || address.is_private()
        || address.is_link_local()
        || address.is_broadcast()
        || address.is_multicast()
        || address.is_documentation()
        // "This network", carrier-grade NAT, the IETF's own block, benchmarking, reserved.
        || first == 0
        || (first == 100 && second & 0xC0 == 64)
        || (first == 192 && second == 0 && third == 0)
        || (first == 198 && second & 0xFE == 18)
        || first >= 240;
    !special
}

fn is_public_v6(address: Ipv6Addr) -> bool {
    let segments = address.segments();
    let carried = |high: u16, low: u16| Ipv4Addr::new((high >> 8) as u8, high as u8, (low >> 8) as u8, low as u8);
    // An IPv4 address inside: mapped (::ffff:a.b.c.d), NAT64 (64:ff9b::a.b.c.d), 6to4 (2002:AABB:CCDD::).
    if let Some(mapped) = address.to_ipv4_mapped() {
        return is_public_v4(mapped);
    }
    if segments[..6] == [0x64, 0xFF9B, 0, 0, 0, 0] {
        return is_public_v4(carried(segments[6], segments[7]));
    }
    if segments[0] == 0x2002 {
        return is_public_v4(carried(segments[1], segments[2]));
    }
    let special = address.is_unspecified()
        || address.is_loopback()
        || address.is_multicast()
        // IPv4-compatible (::a.b.c.d, long retired), unique local, link-local, site-local,
        // NAT64 for local use, Teredo, documentation.
        || segments[..6] == [0, 0, 0, 0, 0, 0]
        || segments[0] & 0xFE00 == 0xFC00
        || segments[0] & 0xFFC0 == 0xFE80
        || segments[0] & 0xFFC0 == 0xFEC0
        || (segments[0] == 0x64 && segments[1] == 0xFF9B && segments[2] == 1)
        || (segments[0] == 0x2001 && segments[1] == 0)
        || (segments[0] == 0x2001 && segments[1] == 0x0DB8);
    !special
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
    // Written by name within the thumbnails' folder, through no link (the downloads' `Folder`):
    // a link put there under a thumbnail's name is replaced, not written through.
    use std::io::Write as _;
    use crate::downloads::transfer;
    let root = path.parent().ok_or("a thumbnail with no folder")?;
    let partial = path.with_extension("part");
    transfer::remove_own(root, &partial);
    let mut file = transfer::open_own(root, &partial, true).map_err(|err| err.to_string())?;
    file.write_all(&bytes).map_err(|err| err.to_string())?;
    drop(file);
    transfer::rename_own(root, &partial, path).map_err(|err| err.to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_addresses_of_the_public_internet_are_public() {
        let public = |address: &str| is_public(address.parse().expect("an address"));
        for address in ["8.8.8.8", "1.1.1.1", "93.184.215.14", "172.32.0.1", "100.128.0.1", "198.20.0.1", "2606:4700:4700::1111", "2a00:1450:4001::200e", "::ffff:8.8.8.8", "64:ff9b::808:808"] {
            assert!(public(address), "{address}");
        }
        for address in [
            // This Mac, the home network, shared and link-local addresses (a cloud's metadata among them).
            "127.0.0.1", "127.8.9.10", "10.0.0.1", "172.16.0.1", "172.31.255.255", "192.168.1.1", "100.64.0.1", "100.127.255.255", "169.254.169.254",
            // Nowhere, everywhere, set aside.
            "0.0.0.0", "0.1.2.3", "224.0.0.251", "255.255.255.255", "240.0.0.1", "192.0.0.8", "192.0.2.1", "198.18.0.1", "198.19.255.255", "203.0.113.5",
            // The same in IPv6, and IPv4 addresses carried inside IPv6 ones.
            "::1", "::", "fe80::1", "fc00::1", "fd12:3456:789a::1", "fec0::1", "ff02::1", "2001:db8::1", "::ffff:127.0.0.1", "::ffff:192.168.1.1", "::192.168.1.1",
            "64:ff9b::7f00:1", "64:ff9b::c0a8:101", "64:ff9b:1::1", "2002:c0a8:101::1", "2002:7f00:1::", "2001:0:4136:e378:8000:63bf:3fff:fdd2",
        ] {
            assert!(!public(address), "{address}");
        }
    }

    /// Needs the network: a host of the public internet is reached as before, redirect and all.
    #[test]
    #[ignore]
    fn a_thumbnail_on_the_internet_is_fetched() {
        let folder = std::env::temp_dir().join(format!("finchgram-thumbnails-real-{}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("a folder");
        let agent = agent();
        // github.com answers this one with a redirect to another host: each is looked up.
        let (address, path) = ("https://github.com/FinchGram.png", folder.join("t.img"));
        fetch_to(&agent, address, &path).unwrap_or_else(|err| panic!("{address}: {err}"));
        assert!(std::fs::metadata(&path).expect("the file").len() > 0);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_thumbnail_on_this_mac_is_not_asked_for() {
        // An address that names this Mac, by number and by name: refused before a connection is
        // made (the listener hears nothing).
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
        let port = listener.local_addr().expect("the address").port();
        listener.set_nonblocking(true).expect("non-blocking");
        let folder = std::env::temp_dir().join(format!("finchgram-thumbnails-test-{}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("a folder");
        let agent = agent();
        for host in ["127.0.0.1", "localhost", "[::1]", "[::ffff:127.0.0.1]"] {
            let fetched = fetch_to(&agent, &format!("http://{host}:{port}/thumbnail.jpg"), &folder.join("t.img"));
            assert!(fetched.is_err(), "{host}");
        }
        assert!(listener.accept().is_err(), "a connection was made");
        assert!(!folder.join("t.img").exists());
        let _ = std::fs::remove_dir_all(&folder);
    }
}
