//! Self-update from GitHub Releases (adopted from Coova Studio).
//!
//! Releases are published in this repository ([`RELEASES_REPO`]), next to the source: one release
//! per version, tagged `v<version>`, with three assets produced by the release workflow: the
//! zipped .app (`FinchGram-<version>-macos-arm64.zip`), `SHA256SUMS`, and `SHA256SUMS.sig`, an
//! Ed25519 signature of `SHA256SUMS` made with the private key only CI has. The same repository
//! also publishes finchgram-tdlib under `tdlib-*` tags; those are never marked "latest", and a
//! release whose tag is not `v<version>` is refused anyway.
//!
//! [`check`] asks GitHub for the newest release. When it is newer than this build, [`install`]
//! verifies the signature with the public keys built in from `release-signing.pub`, downloads the
//! zip, checks its SHA-256 against the signed list, unpacks it, swaps it in for the running bundle
//! and arranges a relaunch. Everything here blocks: call it from a thread, never from the UI thread.
//!
//! The app checks on its own once a day, at a moment picked at random within the day so that
//! installed copies do not all ask GitHub at the same time ([`automatic_check_due`],
//! [`plan_next_automatic_check`]); the user can check any time from the Help menu.
//!
//! Only tools that are part of macOS itself are used (`ditto`, `codesign`, `open`, `sh`), by
//! absolute path; nothing third-party is expected on the user's machine (docs/conventions.md).

use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ed25519_dalek::{Signature, VerifyingKey};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Where releases are published: this repository. scripts/release.sh, scripts/fetch-tdlib.sh and
/// the workflows name the same one.
pub const RELEASES_REPO: &str = "FinchGram/FinchGram";
const SUMS_ASSET: &str = "SHA256SUMS";
const SIGNATURE_ASSET: &str = "SHA256SUMS.sig";
/// Public keys allowed to sign releases; see src/bin/finchgram-release-sign.rs.
const SIGNING_PUBLIC_KEYS: &str = include_str!("../release-signing.pub");

/// The .app asset of a version. The name is derived from the version, never taken from the
/// release, so a signed file list from an old release cannot be replayed for a newer version.
fn zip_name(version: &Version) -> String {
    format!("FinchGram-{version}-macos-arm64.zip")
}
/// This build's version, from Cargo.toml.
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// A published version that is newer than this build.
#[derive(Debug, Clone)]
pub struct Release {
    pub version: Version,
    /// Release notes, as written on GitHub (may be empty).
    pub notes: String,
    zip_url: String,
    zip_name: String,
    zip_size: u64,
    sums_url: String,
    sig_url: String,
}

#[derive(Debug)]
pub enum Check {
    UpToDate,
    Available(Release),
}

/// GitHub's JSON for one release; only the fields we read.
#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    size: u64,
    browser_download_url: String,
}

/// ETag of GitHub's last answer and the body that came with it. A request that gets
/// "304 Not Modified" back does not count against GitHub's rate limit, so polling is cheap.
static LAST_ANSWER: Mutex<Option<(String, String)>> = Mutex::new(None);

fn agent(global_timeout: Option<Duration>) -> ureq::Agent {
    ureq::Agent::config_builder()
        .user_agent(format!("finchgram/{CURRENT_VERSION}"))
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_global(global_timeout)
        .build()
        .into()
}

/// Ask GitHub for the newest release and compare it with this build.
pub fn check() -> Result<Check, String> {
    let url = format!("https://api.github.com/repos/{RELEASES_REPO}/releases/latest");
    let cached = LAST_ANSWER.lock().unwrap().clone();

    let mut request = agent(Some(Duration::from_secs(20)))
        .get(url.as_str())
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28");
    if let Some((etag, _)) = &cached {
        request = request.header("If-None-Match", etag);
    }

    let body = match request.call() {
        // "304 Not Modified" is a success for ureq (only 4xx and 5xx are errors), and it comes
        // with a Content-Encoding header but no body: reading it as gzip fails. Use the cache.
        Ok(response) if response.status().as_u16() == 304 => match cached {
            Some((_, body)) => body,
            None => return Err("GitHub answered 304 to a first request".to_string()),
        },
        Ok(mut response) => {
            let etag = response
                .headers()
                .get("etag")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            let body = response
                .body_mut()
                .read_to_string()
                .map_err(|err| format!("cannot read GitHub's answer: {err}"))?;
            if let Some(etag) = etag {
                *LAST_ANSWER.lock().unwrap() = Some((etag, body.clone()));
            }
            body
        }
        // No release published yet: nothing to update to.
        Err(ureq::Error::StatusCode(404)) => return Ok(Check::UpToDate),
        Err(ureq::Error::StatusCode(403 | 429)) => {
            return Err("GitHub is rate-limiting update checks; try again in a few minutes".to_string());
        }
        Err(ureq::Error::StatusCode(code)) => return Err(format!("GitHub answered HTTP {code}")),
        Err(err) => return Err(format!("cannot reach GitHub: {err}")),
    };

    let release = parse(&body)?;
    let current = Version::parse(CURRENT_VERSION)
        .map_err(|err| format!("this build's version {CURRENT_VERSION:?} is invalid: {err}"))?;
    Ok(if release.version > current {
        Check::Available(release)
    } else {
        Check::UpToDate
    })
}

fn parse(body: &str) -> Result<Release, String> {
    let api: ApiRelease =
        serde_json::from_str(body).map_err(|err| format!("unexpected answer from GitHub: {err}"))?;
    let tag_version = api
        .tag_name
        .strip_prefix('v')
        .ok_or_else(|| format!("the latest release {:?} is not an app release (v<version>)", api.tag_name))?;
    let version = Version::parse(tag_version)
        .map_err(|err| format!("release tag {:?} is not a version: {err}", api.tag_name))?;
    let asset = |name: &str| {
        api.assets
            .iter()
            .find(|asset| asset.name == name)
            .ok_or_else(|| format!("release {} has no {name} asset", api.tag_name))
    };
    let zip = asset(&zip_name(&version))?;
    let sums = asset(SUMS_ASSET)?;
    let sig = asset(SIGNATURE_ASSET)?;
    Ok(Release {
        version,
        // The workflow puts a footer about the assets after a "---" line; not for the UI.
        notes: api
            .body
            .unwrap_or_default()
            .split("\n---")
            .next()
            .unwrap_or_default()
            .trim()
            .to_string(),
        zip_url: zip.browser_download_url.clone(),
        zip_name: zip.name.clone(),
        zip_size: zip.size,
        sums_url: sums.browser_download_url.clone(),
        sig_url: sig.browser_download_url.clone(),
    })
}

/// The .app the running executable belongs to, if it is one that can be replaced.
pub fn installed_bundle() -> Result<PathBuf, String> {
    let exe = std::env::current_exe()
        .map_err(|err| format!("cannot locate the running executable: {err}"))?;
    // <name>.app/Contents/MacOS/<exe>
    let bundle = exe
        .ancestors()
        .nth(3)
        .filter(|dir| dir.extension().is_some_and(|ext| ext == "app"))
        .ok_or_else(|| {
            "not running from an installed .app (development build), so the update cannot be installed"
                .to_string()
        })?;
    if bundle.to_string_lossy().contains("/AppTranslocation/") {
        return Err("macOS is running this app from a temporary location. Move FinchGram into the \
                    Applications folder, open it from there, then update."
            .to_string());
    }
    Ok(bundle.to_path_buf())
}

/// Download `release`, verify it, swap it in for the running bundle and arrange the relaunch.
/// `progress` is called with 0..1 while downloading. On success the caller must quit the event
/// loop: the new copy starts as soon as this process has exited.
pub fn install(release: &Release, progress: &dyn Fn(f32)) -> Result<(), String> {
    let app = installed_bundle()?;

    let work = dirs::cache_dir()
        .ok_or_else(|| "no cache directory".to_string())?
        .join("FinchGram")
        .join("update");
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).map_err(|err| format!("cannot create {}: {err}", work.display()))?;

    // The signed file list first: nothing is downloaded on the strength of an unsigned release.
    let sums = fetch_bytes(&release.sums_url)?;
    let signature = fetch_text(&release.sig_url)?;
    verify_signature(&sums, &signature, &public_keys(SIGNING_PUBLIC_KEYS))?;
    let sums = String::from_utf8_lossy(&sums);

    let zip = work.join(&release.zip_name);
    download(&release.zip_url, &zip, release.zip_size, progress)?;
    verify_sha256(&zip, &release.zip_name, &sums)?;

    let unpacked = work.join("unpacked");
    fs::create_dir_all(&unpacked).map_err(|err| format!("cannot create {}: {err}", unpacked.display()))?;
    run("/usr/bin/ditto", [os("-x"), os("-k"), zip.as_os_str().to_owned(), unpacked.as_os_str().to_owned()])?;
    let new_app = find_app(&unpacked)?;
    check_bundle(&new_app)?;

    let old = swap(&app, &new_app)?;
    relaunch(&app, &old)?;
    let _ = fs::remove_dir_all(&work);
    Ok(())
}

fn download(url: &str, dest: &Path, expected_size: u64, progress: &dyn Fn(f32)) -> Result<(), String> {
    let mut response = agent(None)
        .get(url)
        .call()
        .map_err(|err| format!("download failed: {err}"))?;
    let total = response.body_mut().content_length().unwrap_or(expected_size).max(1);
    let mut reader = response.body_mut().as_reader();
    let mut file = File::create(dest).map_err(|err| format!("cannot create {}: {err}", dest.display()))?;

    let mut buf = [0u8; 64 * 1024];
    let mut done: u64 = 0;
    let mut reported = -1.0f32;
    loop {
        let n = reader.read(&mut buf).map_err(|err| format!("download interrupted: {err}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|err| format!("cannot write {}: {err}", dest.display()))?;
        done += n as u64;
        let fraction = (done as f32 / total as f32).min(1.0);
        if fraction - reported >= 0.01 {
            reported = fraction;
            progress(fraction);
        }
    }
    progress(1.0);
    Ok(())
}

fn fetch_text(url: &str) -> Result<String, String> {
    agent(Some(Duration::from_secs(20)))
        .get(url)
        .call()
        .map_err(|err| format!("cannot download {url}: {err}"))?
        .body_mut()
        .read_to_string()
        .map_err(|err| format!("cannot read {url}: {err}"))
}

/// The exact bytes of a small file; what a signature was made over.
fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    agent(Some(Duration::from_secs(20)))
        .get(url)
        .call()
        .map_err(|err| format!("cannot download {url}: {err}"))?
        .body_mut()
        .read_to_vec()
        .map_err(|err| format!("cannot read {url}: {err}"))
}

/// The keys listed in release-signing.pub: one hex key per line, `#` comments and blanks
/// ignored. (Kept identical to the copy in src/bin/finchgram-release-sign.rs.)
fn public_keys(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect()
}

/// `signature_hex` must be an Ed25519 signature of `message` by one of `keys_hex`.
/// (Kept identical to the copy in src/bin/finchgram-release-sign.rs.)
fn verify_signature(message: &[u8], signature_hex: &str, keys_hex: &[&str]) -> Result<(), String> {
    let bytes = hex::decode(signature_hex.trim()).map_err(|err| format!("{SIGNATURE_ASSET} is not hex: {err}"))?;
    let signature = Signature::from_slice(&bytes)
        .map_err(|err| format!("{SIGNATURE_ASSET} is not an Ed25519 signature: {err}"))?;
    if keys_hex.is_empty() {
        return Err("this build has no release signing key built in".to_string());
    }
    for key_hex in keys_hex {
        let key_bytes: [u8; 32] = hex::decode(key_hex)
            .ok()
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or_else(|| "a built-in release signing key is malformed".to_string())?;
        let key = VerifyingKey::from_bytes(&key_bytes)
            .map_err(|err| format!("a built-in release signing key is invalid: {err}"))?;
        if key.verify_strict(message, &signature).is_ok() {
            return Ok(());
        }
    }
    Err("the release is not signed with FinchGram's release key; refusing to install it".to_string())
}

/// `sums` is in `shasum -a 256` format: one "<hex>  <file name>" per line.
fn verify_sha256(file: &Path, name: &str, sums: &str) -> Result<(), String> {
    let expected = sums
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let hash = parts.next()?;
            let entry = parts.next()?.trim_start_matches('*');
            (entry == name).then(|| hash.to_ascii_lowercase())
        })
        .next()
        .ok_or_else(|| format!("{SUMS_ASSET} has no entry for {name}"))?;

    let mut input = File::open(file).map_err(|err| format!("cannot open {}: {err}", file.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = input.read(&mut buf).map_err(|err| format!("cannot read {}: {err}", file.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let actual: String = hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect();
    if actual != expected {
        return Err(format!("SHA-256 mismatch for {name}: expected {expected}, got {actual}"));
    }
    Ok(())
}

/// The one .app inside the unpacked archive.
fn find_app(dir: &Path) -> Result<PathBuf, String> {
    let entries = fs::read_dir(dir).map_err(|err| format!("cannot list {}: {err}", dir.display()))?;
    let mut apps: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_dir() && path.extension().is_some_and(|ext| ext == "app"))
        .collect();
    match apps.len() {
        1 => Ok(apps.remove(0)),
        n => Err(format!("expected one .app in the archive, found {n}")),
    }
}

/// The downloaded bundle is complete and its signature is intact.
fn check_bundle(app: &Path) -> Result<(), String> {
    for name in ["finchgram", "finchgram-tdlib"] {
        let path = app.join("Contents").join("MacOS").join(name);
        if !path.is_file() {
            return Err(format!("the downloaded app is missing Contents/MacOS/{name}"));
        }
    }
    run("/usr/bin/codesign", [os("--verify"), os("--deep"), os("--strict"), app.as_os_str().to_owned()])
        .map_err(|err| format!("the downloaded app failed signature verification: {err}"))
}

/// Put `new_app` where `app` is. The running bundle is moved aside (same directory, so this is
/// a rename and can be undone), never modified in place: macOS kills a process whose executable
/// changes under it. Returns where the old bundle went; the relaunch helper deletes it.
fn swap(app: &Path, new_app: &Path) -> Result<PathBuf, String> {
    let name = app
        .file_name()
        .ok_or_else(|| format!("bad app path {}", app.display()))?
        .to_string_lossy()
        .into_owned();
    let old = app.with_file_name(format!("{name}.old"));
    let _ = fs::remove_dir_all(&old);
    fs::rename(app, &old)
        .map_err(|err| format!("cannot move the current app aside ({}): {err}", app.display()))?;
    if let Err(err) = move_dir(new_app, app) {
        let _ = fs::rename(&old, app);
        return Err(format!("cannot put the new app into {}: {err}", app.display()));
    }
    Ok(old)
}

/// Rename, or copy when `from` and `to` are on different volumes.
fn move_dir(from: &Path, to: &Path) -> Result<(), String> {
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    run("/usr/bin/ditto", [from.as_os_str().to_owned(), to.as_os_str().to_owned()])?;
    let _ = fs::remove_dir_all(from);
    Ok(())
}

/// Once this process is gone: delete the old bundle and open the new one. A detached shell does
/// it so that it survives our exit. (`open` while we are still running would only bring this
/// window to the front, hence the wait.)
fn relaunch(app: &Path, old: &Path) -> Result<(), String> {
    const SCRIPT: &str = r#"while /bin/kill -0 "$1" 2>/dev/null; do /bin/sleep 0.2; done; /bin/rm -rf "$2"; exec /usr/bin/open "$3""#;
    Command::new("/bin/sh")
        .arg("-c")
        .arg(SCRIPT)
        .arg("finchgram-relaunch")
        .arg(std::process::id().to_string())
        .arg(old)
        .arg(app)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|err| format!("cannot start the relaunch helper: {err}"))?;
    Ok(())
}

fn os(s: &str) -> std::ffi::OsString {
    s.into()
}

/// Run a macOS tool; non-zero exit is an error carrying its stderr.
fn run(program: &str, args: impl IntoIterator<Item = std::ffi::OsString>) -> Result<(), String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|err| format!("cannot run {program}: {err}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("{program} failed ({}): {}", output.status, stderr.trim()))
    }
}

// ---- when to check on our own -------------------------------------------------------------

const DAY: u64 = 24 * 60 * 60;
const HOUR: u64 = 60 * 60;

/// Kept in `update-check.toml` next to settings.toml, so the plan survives restarts.
#[derive(Serialize, Deserialize, Default)]
struct Schedule {
    /// Unix time, in seconds, of the next automatic check; 0 when none has been planned yet.
    #[serde(default)]
    next_check_at: u64,
}

fn schedule_file() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("FinchGram").join("update-check.toml"))
}

fn load_schedule() -> Schedule {
    schedule_file()
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|text| toml::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_schedule(schedule: &Schedule) {
    let Some(path) = schedule_file() else { return };
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Ok(text) = toml::to_string(schedule)
        && let Err(err) = fs::write(&path, text)
    {
        eprintln!("update: cannot write {}: {err}", path.display());
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// A random number of seconds below `limit`; the standard library's hasher is seeded from the
/// operating system's randomness, which is all this needs.
fn random_below(limit: u64) -> u64 {
    use std::hash::{BuildHasher, Hasher};
    std::collections::hash_map::RandomState::new().build_hasher().finish() % limit
}

/// Whether the daily automatic check is due now. The very first call plans the first check for
/// a random moment within the next 24 hours and answers false.
pub fn automatic_check_due() -> bool {
    let mut schedule = load_schedule();
    if schedule.next_check_at == 0 {
        schedule.next_check_at = now() + random_below(DAY);
        save_schedule(&schedule);
        return false;
    }
    now() >= schedule.next_check_at
}

/// After a check, manual or automatic: the next one happens at a random moment of the following
/// day (days counted in UTC; only the spread matters).
pub fn plan_next_automatic_check() {
    let now = now();
    save_schedule(&Schedule { next_check_at: now - now % DAY + DAY + random_below(DAY) });
}

/// After a check that could not reach GitHub: try again in one to two hours rather than tomorrow.
pub fn plan_retry_after_failure() {
    save_schedule(&Schedule { next_check_at: now() + HOUR + random_below(HOUR) });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release_json(tag: &str, assets: &[&str]) -> String {
        let assets: Vec<String> = assets
            .iter()
            .map(|name| format!(r#"{{"name":"{name}","size":1,"browser_download_url":"https://example.org/{name}"}}"#))
            .collect();
        format!(r#"{{"tag_name":"{tag}","body":"Notes.\n---\nfooter","assets":[{}]}}"#, assets.join(","))
    }

    #[test]
    fn an_app_release_with_its_three_assets_is_read() {
        let body = release_json("v0.2.0", &["FinchGram-0.2.0-macos-arm64.zip", "SHA256SUMS", "SHA256SUMS.sig"]);
        let release = parse(&body).unwrap();
        assert_eq!(release.version, Version::new(0, 2, 0));
        assert_eq!(release.notes, "Notes.");
    }

    #[test]
    fn a_vendor_release_is_never_taken_for_the_app() {
        let body = release_json("tdlib-1.8.67-1", &["finchgram-tdlib-1.8.67-macos-arm64.tar.gz", "SHA256SUMS"]);
        assert!(parse(&body).is_err());
    }

    #[test]
    fn a_release_without_its_signature_is_refused() {
        let body = release_json("v0.2.0", &["FinchGram-0.2.0-macos-arm64.zip", "SHA256SUMS"]);
        assert!(parse(&body).is_err());
    }
}
