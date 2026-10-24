//! The app's own database: a plain SQLite file, `finchgram.db` next to settings.toml, that any
//! SQLite tool opens, since the user's data is the user's (the maintainer's word, 2026-10-24).
//! It holds what FinchGram keeps that is neither a preference (settings.toml is for those, and
//! only those) nor Telegram's (that is TDLib's database): the servers of the downloads page with
//! their passwords, the programs the user has allowed or turned away at AI access, and the keys
//! of the SSH servers the user trusted, with when. Only the password column is sealed
//! (AES-256-GCM, `cipher` below) under
//! a random key made the first time and kept in the Keychain under FinchGram's name
//! (platform::KEYCHAIN_SERVICE, the account "database"), where only this app reads it; every
//! other column is in the clear. TDLib's database key is kept the same way (src/telegram/mod.rs).
//! A password is sealed to its server (protocol, address, port and account are the seal's
//! additional data), so that the sealed text copied into a row that names another server does
//! not open there: a program that can write the file could otherwise point a server at itself and
//! have the app send it the password. The rows of the programs allowed carry a tag under the same
//! key (`seal`), so that a row written by anything but FinchGram, or changed since, is not
//! believed: the user is asked about the program as if it were new.
//!
//! The layout is numbered: SQLite's `PRAGMA user_version` holds LAYOUT, counted up with every
//! change to a table, each change with its step in `upgrade`. A file from an older FinchGram is
//! first copied as it is into ~/Documents/FinchGram/backup/, then brought up step by step, each
//! step one transaction, with no word to the user; a file from a newer FinchGram is refused, so
//! that the newer layout stays as it is. The file of 0.5.9 and before was encrypted as a whole
//! (SQLCipher): it is moved into that backup folder and a new file is made, at the maintainer's
//! word of 2026-10-24 ("就当作我没安装过以前的 FinchGram"). One connection, on the UI thread.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use rusqlite::{Connection, OptionalExtension as _, params};

use crate::platform;

/// The Keychain account of the key that seals the passwords.
const KEY_ACCOUNT: &str = "database";

/// The layout this FinchGram makes and expects: counted up with every change to a table, each
/// with its step in `upgrade`.
///
/// 1 (0.6.0): `servers` (protocol, address, port; the password sealed), `mcp_apps`, `ssh_hosts`.
/// 2 (after 0.6.2): the password sealed to its server (v2 of `cipher`), the passwords of layout 1
///    sealed again that way; `mcp_apps.seal`, the row's tag, the rows of before dropped (every
///    program asks again, once); `ssh_hosts.first_seen`, when the user trusted the key (0 for one
///    that 0.6.2 or before took on its own, without asking).
pub const LAYOUT: i32 = 2;

/// The tables of layout 1, as 0.6.0 made them.
const TABLES_OF_LAYOUT_1: &str = "CREATE TABLE IF NOT EXISTS servers (
         id INTEGER PRIMARY KEY,
         position INTEGER NOT NULL,
         name TEXT NOT NULL,
         protocol TEXT NOT NULL,
         address TEXT NOT NULL,
         port INTEGER NOT NULL,
         username TEXT NOT NULL,
         password TEXT NOT NULL
     );
     CREATE TABLE IF NOT EXISTS mcp_apps (
         key TEXT PRIMARY KEY,
         path TEXT NOT NULL,
         name TEXT NOT NULL,
         allowed INTEGER NOT NULL,
         last_used INTEGER NOT NULL
     );
     CREATE TABLE IF NOT EXISTS ssh_hosts (
         host TEXT NOT NULL,
         port INTEGER NOT NULL,
         key TEXT NOT NULL,
         PRIMARY KEY (host, port)
     );";

/// What a plain SQLite file starts with.
const SQLITE_HEADER: &[u8; 16] = b"SQLite format 3\0";

thread_local! {
    static DATABASE: RefCell<Option<Connection>> = const { RefCell::new(None) };
    static KEY: RefCell<Option<[u8; 32]>> = const { RefCell::new(None) };
}

/// A server of the downloads page, as the user named it (docs/downloads.md).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerRecord {
    pub name: String,
    /// "webdav", "ftp", "ftps" or "sftp".
    pub protocol: String,
    /// As typed: "https://nas.home/dav" for WebDAV (a folder may be part of it), "host" or
    /// "host/folder" for the others; never with a port in it, that is `port`.
    pub address: String,
    /// 0 for the protocol's default.
    pub port: u16,
    /// The account; "" for a server that asks for none.
    pub username: String,
    /// In the clear here; sealed in the file.
    pub password: String,
}

impl ServerRecord {
    /// What the password is sealed to: the server it is for.
    fn binding(&self) -> String {
        binding(&self.protocol, &self.address, self.port, &self.username)
    }
}

/// The seal's additional data for a server's password: its protocol, address, port and account,
/// so that the sealed text does not open in a row that names any other server.
fn binding(protocol: &str, address: &str, port: u16, username: &str) -> String {
    format!("{protocol}\0{address}\0{port}\0{username}")
}

/// A program that connected to the MCP server, and what the user answered when it first did
/// (src/mcp/access.rs).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpApp {
    /// What it is remembered by (`Host::key`, src/mcp/peer.rs): its code signature
    /// ("Q6L2SF6YDW/com.anthropic.claude-code"), the same for every version and copy of it, or
    /// where it is when it has none from a team.
    pub key: String,
    /// Where the system last saw it: an app bundle ("/Applications/Claude.app") or a program.
    pub path: String,
    /// Its name as shown: the bundle's, or the program's file name.
    pub name: String,
    pub allowed: bool,
    /// When it last connected or asked for something, in seconds since the epoch; 0 for never.
    pub last_used: i64,
}

impl McpApp {
    /// What the row's tag is over: what it is remembered by, and the answer. Where it is, its
    /// name and when it was last used are only shown.
    fn sealed_text(&self) -> String {
        format!("mcp_app\0{}\0{}", self.key, u8::from(self.allowed))
    }
}

/// Open the database, making it and its key the first time, and bringing an older file up to
/// LAYOUT. Call once, on the UI thread, before anything reads it. Err when it cannot be opened:
/// the Keychain gave no key, the file is not a database, or it was made by a newer FinchGram;
/// then nothing of it is read or written, and the error says why.
pub fn open() -> Result<(), String> {
    let path = file().ok_or("the system has no folder for FinchGram's database")?;
    let key = key()?;
    let backup = backup_folder().ok_or("the system has no Documents folder for FinchGram's backups")?;
    let connection = open_at(&path, &key, &backup)?;
    DATABASE.with(|slot| *slot.borrow_mut() = Some(connection));
    KEY.with(|slot| *slot.borrow_mut() = Some(key));
    Ok(())
}

/// Where the database is: next to settings.toml (src/settings.rs).
pub fn file() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("FinchGram").join("finchgram.db"))
}

/// Where copies of the file go before a layout change: ~/Documents/FinchGram/backup.
pub fn backup_folder() -> Option<PathBuf> {
    Some(dirs::document_dir().or_else(dirs::home_dir)?.join("FinchGram").join("backup"))
}

/// The key from the Keychain, or a new one kept there: 32 bytes.
fn key() -> Result<[u8; 32], String> {
    let encoded = match platform::keychain_read(platform::KEYCHAIN_SERVICE, KEY_ACCOUNT) {
        Ok(Some(key)) if !key.is_empty() => key,
        Ok(_) => {
            let key = platform::random_key().ok_or("the system gave no random bytes for a key")?;
            platform::keychain_store(platform::KEYCHAIN_SERVICE, KEY_ACCOUNT, &key).map_err(|err| format!("cannot keep the database key in the Keychain: {err}"))?;
            key
        }
        Err(err) => return Err(format!("cannot read the database key from the Keychain: {err}")),
    };
    decode_key(&encoded)
}

fn decode_key(encoded: &str) -> Result<[u8; 32], String> {
    let raw = base64::engine::general_purpose::STANDARD.decode(encoded).map_err(|err| format!("the database key is not base64: {err}"))?;
    raw.try_into().map_err(|raw: Vec<u8>| format!("the database key is {} bytes, not 32", raw.len()))
}

/// `path` opened, made when it is not there, brought up to LAYOUT when it is older (a copy of it
/// in `backup` first), refused when it is newer or not a database.
fn open_at(path: &Path, key: &[u8; 32], backup: &Path) -> Result<Connection, String> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|err| format!("cannot create {}: {err}", folder.display()))?;
    }
    put_aside_if_encrypted(path, backup)?;
    let connection = Connection::open(path).map_err(|err| format!("cannot open {}: {err}", path.display()))?;
    connection.execute_batch("PRAGMA journal_mode = WAL;").map_err(|err| format!("cannot set up {}: {err}", path.display()))?;
    let version: i32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0)).map_err(|err| format!("{} cannot be read: {err}", path.display()))?;
    let tables: i64 = connection
        .query_row("SELECT count(*) FROM sqlite_master WHERE type = 'table'", [], |row| row.get(0))
        .map_err(|err| format!("{} cannot be read: is it a database? ({err})", path.display()))?;
    if tables == 0 {
        // A new file: the newest layout at once.
        upgrade(&connection, 0, key)?;
        return Ok(connection);
    }
    if version > LAYOUT {
        return Err(format!("{} was made by a newer FinchGram (layout {version}; this one knows {LAYOUT}): update FinchGram to use it", path.display()));
    }
    if version < LAYOUT {
        copy_aside(&connection, path, backup, &format!("layout-{version}"))?;
        upgrade(&connection, version, key)?;
    }
    Ok(connection)
}

/// The file of 0.5.9 and before, encrypted as a whole: moved into `backup` with its journal,
/// so that a new file is made in its place. Nothing happens to a plain file, or to none.
fn put_aside_if_encrypted(path: &Path, backup: &Path) -> Result<(), String> {
    let Ok(mut file) = std::fs::File::open(path) else { return Ok(()) };
    let mut header = [0u8; 16];
    let read = std::io::Read::read(&mut file, &mut header).map_err(|err| format!("cannot read {}: {err}", path.display()))?;
    drop(file);
    if read == 0 || (read == 16 && &header == SQLITE_HEADER) {
        return Ok(());
    }
    std::fs::create_dir_all(backup).map_err(|err| format!("cannot create {}: {err}", backup.display()))?;
    let stamp = chrono::Local::now().format("%Y-%m-%d-%H%M%S");
    let name = format!("finchgram-encrypted-before-0.6.0-{stamp}.db");
    let target = backup.join(&name);
    std::fs::rename(path, &target).map_err(|err| format!("cannot move {} to {}: {err}", path.display(), target.display()))?;
    for suffix in ["-wal", "-shm"] {
        let journal = with_suffix(path, suffix);
        if journal.exists() {
            let _ = std::fs::rename(&journal, with_suffix(&target, suffix));
        }
    }
    eprintln!("database: {} was encrypted as a whole (FinchGram 0.5.9 or before): moved to {}", path.display(), target.display());
    Ok(())
}

/// A copy of the file as it is, whole (the journal folded in first), into `backup`, named after
/// what it is: "finchgram-layout-1-2026-10-24-183000.db".
fn copy_aside(connection: &Connection, path: &Path, backup: &Path, what: &str) -> Result<(), String> {
    connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").map_err(|err| format!("cannot fold the journal of {}: {err}", path.display()))?;
    std::fs::create_dir_all(backup).map_err(|err| format!("cannot create {}: {err}", backup.display()))?;
    let stamp = chrono::Local::now().format("%Y-%m-%d-%H%M%S");
    let target = backup.join(format!("finchgram-{what}-{stamp}.db"));
    std::fs::copy(path, &target).map_err(|err| format!("cannot copy {} to {}: {err}", path.display(), target.display()))?;
    eprintln!("database: a copy of {} is at {}", path.display(), target.display());
    Ok(())
}

/// The layout brought from `from` up to LAYOUT, one step at a time, each a transaction that ends
/// by stamping the version; `key` is the Keychain's, for a step that seals something again.
fn upgrade(connection: &Connection, from: i32, key: &[u8; 32]) -> Result<(), String> {
    for to in (from + 1)..=LAYOUT {
        let step = || -> Result<(), String> {
            match to {
                1 => connection.execute_batch(TABLES_OF_LAYOUT_1).map_err(|err| err.to_string()),
                2 => {
                    // The programs' rows get their tag. The rows of before were open to anything
                    // that could write the file, so none of them is carried over: each program
                    // asks again, once. The passwords are sealed to their servers. The SSH
                    // servers' keys get the day they were trusted; the ones of before keep 0.
                    connection
                        .execute_batch(
                            "ALTER TABLE mcp_apps ADD COLUMN seal TEXT NOT NULL DEFAULT ''; DELETE FROM mcp_apps;
                             ALTER TABLE ssh_hosts ADD COLUMN first_seen INTEGER NOT NULL DEFAULT 0;",
                        )
                        .map_err(|err| err.to_string())?;
                    reseal_servers(connection, key)
                }
                _ => Err(format!("no step to layout {to}")),
            }
        };
        connection.execute_batch("BEGIN;").map_err(|err| format!("cannot bring the database to layout {to}: {err}"))?;
        let stamped = step().and_then(|()| connection.execute_batch(&format!("PRAGMA user_version = {to}; COMMIT;")).map_err(|err| err.to_string()));
        if let Err(err) = stamped {
            let _ = connection.execute_batch("ROLLBACK;");
            return Err(format!("cannot bring the database to layout {to}: {err}"));
        }
    }
    Ok(())
}

/// The passwords of layout 1, sealed to nothing, sealed again to their servers (layout 2). One
/// that does not open (sealed under a key the Keychain no longer has) is left empty: the user
/// types it again, and the copy in the backup folder has it as it was.
fn reseal_servers(connection: &Connection, key: &[u8; 32]) -> Result<(), String> {
    let mut statement = connection.prepare("SELECT id, protocol, address, port, username, password FROM servers").map_err(|err| err.to_string())?;
    let rows: Vec<(i64, String, String, i64, String, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)))
        .and_then(Iterator::collect)
        .map_err(|err| err.to_string())?;
    for (id, protocol, address, port, username, sealed) in rows {
        let password = cipher::open_unbound(key, &sealed).unwrap_or_else(|err| {
            eprintln!("database: the password of {address} cannot be carried over ({err}): type it again");
            String::new()
        });
        let resealed = cipher::seal(key, &binding(&protocol, &address, u16::try_from(port).unwrap_or(0), &username), &password)?;
        connection.execute("UPDATE servers SET password = ?1 WHERE id = ?2", params![resealed, id]).map_err(|err| err.to_string())?;
    }
    Ok(())
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

/// Something with the open database; none when it is not open.
fn with<R>(work: impl FnOnce(&Connection) -> rusqlite::Result<R>) -> Result<R, String> {
    DATABASE.with(|slot| {
        let slot = slot.borrow();
        let connection = slot.as_ref().ok_or("the database is not open")?;
        work(connection).map_err(|err| err.to_string())
    })
}

/// The key the passwords are sealed with, once the database is open.
fn sealing_key() -> Option<[u8; 32]> {
    KEY.with(|slot| *slot.borrow())
}

// ---- the passwords -----------------------------------------------------------------------------

/// The password column: AES-256-GCM under the Keychain's key, a fresh nonce each time, the
/// server it is for (`binding`) as the additional data, written as base64 of nonce + ciphertext
/// after "v2:". "" stays "". A tool opening the file sees the column as that text; the app shows
/// the password in the server's form. Layout 1 sealed to nothing, after "v1:": those open only
/// for the step that seals them again. The tag of a program's row (`tag`) is HMAC-SHA256 under a
/// key drawn from the same one (HKDF), base64.
mod cipher {
    use base64::Engine as _;
    use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
    use ring::rand::{SecureRandom as _, SystemRandom};
    use ring::{hkdf, hmac};

    const PREFIX: &str = "v2:";
    const PREFIX_UNBOUND: &str = "v1:";
    /// What tells the tags' key from the sealing key.
    const TAG_KEY_PURPOSE: &[u8] = b"FinchGram database row tag";

    pub fn seal(key: &[u8; 32], binding: &str, password: &str) -> Result<String, String> {
        if password.is_empty() {
            return Ok(String::new());
        }
        let sealer = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).map_err(|_| "the sealing key is not one")?);
        let mut nonce = [0u8; NONCE_LEN];
        SystemRandom::new().fill(&mut nonce).map_err(|_| "the system gave no random bytes for a nonce")?;
        let mut data = password.as_bytes().to_vec();
        sealer.seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::from(binding.as_bytes()), &mut data).map_err(|_| "sealing the password failed")?;
        let mut whole = nonce.to_vec();
        whole.extend_from_slice(&data);
        Ok(format!("{PREFIX}{}", base64::engine::general_purpose::STANDARD.encode(whole)))
    }

    /// The password sealed to `binding`; none that was sealed to another server, or to nothing.
    pub fn open(key: &[u8; 32], binding: &str, sealed: &str) -> Result<String, String> {
        if sealed.is_empty() {
            return Ok(String::new());
        }
        if sealed.starts_with(PREFIX_UNBOUND) {
            return Err("the password was sealed by FinchGram 0.6.2 or before, to no server; it is not opened".into());
        }
        let encoded = sealed.strip_prefix(PREFIX).ok_or("the sealed password does not start with v2:")?;
        open_encoded(key, binding.as_bytes(), encoded).map_err(|err| if err == KEY_OR_BINDING { "the password was sealed with another key, or to another server".into() } else { err })
    }

    /// A password of layout 1, sealed to nothing: for the step that seals it again, and nothing
    /// else.
    pub fn open_unbound(key: &[u8; 32], sealed: &str) -> Result<String, String> {
        if sealed.is_empty() {
            return Ok(String::new());
        }
        let encoded = sealed.strip_prefix(PREFIX_UNBOUND).ok_or("the sealed password does not start with v1:")?;
        open_encoded(key, &[], encoded).map_err(|err| if err == KEY_OR_BINDING { "the password was sealed with another key".into() } else { err })
    }

    const KEY_OR_BINDING: &str = "key or binding";

    fn open_encoded(key: &[u8; 32], binding: &[u8], encoded: &str) -> Result<String, String> {
        let mut whole = base64::engine::general_purpose::STANDARD.decode(encoded).map_err(|err| format!("the sealed password is not base64: {err}"))?;
        if whole.len() < NONCE_LEN {
            return Err("the sealed password is too short".into());
        }
        let data = whole.split_off(NONCE_LEN);
        let nonce: [u8; NONCE_LEN] = whole.try_into().map_err(|_| "the nonce is not one")?;
        let opener = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).map_err(|_| "the sealing key is not one")?);
        let mut data = data;
        let plain = opener.open_in_place(Nonce::assume_unique_for_key(nonce), Aad::from(binding), &mut data).map_err(|_| KEY_OR_BINDING)?;
        String::from_utf8(plain.to_vec()).map_err(|_| "the password is not text".into())
    }

    /// A password sealed as layout 1 sealed it, to nothing: for the test of the step that seals
    /// them again.
    #[cfg(test)]
    pub fn seal_unbound(key: &[u8; 32], password: &str) -> String {
        let sealer = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).expect("a key"));
        let nonce = [7u8; NONCE_LEN];
        let mut data = password.as_bytes().to_vec();
        sealer.seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::empty(), &mut data).expect("sealed");
        let mut whole = nonce.to_vec();
        whole.extend_from_slice(&data);
        format!("{PREFIX_UNBOUND}{}", base64::engine::general_purpose::STANDARD.encode(whole))
    }

    /// The key the tags are under: drawn from the sealing key, so that the two never meet.
    fn tag_key(key: &[u8; 32]) -> hmac::Key {
        let mut drawn = [0u8; 32];
        hkdf::Salt::new(hkdf::HKDF_SHA256, TAG_KEY_PURPOSE)
            .extract(key)
            .expand(&[], hkdf::HKDF_SHA256)
            .and_then(|okm| okm.fill(&mut drawn))
            .expect("32 bytes are within HKDF-SHA256's reach");
        hmac::Key::new(hmac::HMAC_SHA256, &drawn)
    }

    /// The tag of `text` under `key`: what a row of FinchGram's carries.
    pub fn tag(key: &[u8; 32], text: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(hmac::sign(&tag_key(key), text.as_bytes()).as_ref())
    }

    /// Whether `tag` is the tag of `text` under `key`: the row is FinchGram's, as it was written.
    pub fn tag_matches(key: &[u8; 32], text: &str, tag: &str) -> bool {
        let Ok(tag) = base64::engine::general_purpose::STANDARD.decode(tag) else { return false };
        hmac::verify(&tag_key(key), text.as_bytes(), &tag).is_ok()
    }
}

// ---- the servers -------------------------------------------------------------------------------

/// The servers of the downloads page, in the order the user keeps them, their passwords opened.
pub fn servers() -> Vec<ServerRecord> {
    let Some(key) = sealing_key() else { return Vec::new() };
    match with(|connection| read_servers(connection, &key)) {
        Ok(servers) => servers,
        Err(err) => {
            eprintln!("database: cannot read the servers: {err}");
            Vec::new()
        }
    }
}

/// Keep `servers` as the servers, in this order, in place of the ones kept before.
pub fn save_servers(servers: &[ServerRecord]) -> Result<(), String> {
    let key = sealing_key().ok_or("the database is not open")?;
    let mut sealed = Vec::with_capacity(servers.len());
    for server in servers {
        sealed.push(ServerRecord { password: cipher::seal(&key, &server.binding(), &server.password)?, ..server.clone() });
    }
    with(|connection| write_servers(connection, &sealed))
}

fn read_servers(connection: &Connection, key: &[u8; 32]) -> rusqlite::Result<Vec<ServerRecord>> {
    let mut statement = connection.prepare("SELECT name, protocol, address, port, username, password FROM servers ORDER BY position, id")?;
    let rows = statement.query_map([], |row| {
        let mut server = ServerRecord {
            name: row.get(0)?,
            protocol: row.get(1)?,
            address: row.get(2)?,
            port: u16::try_from(row.get::<_, i64>(3)?).unwrap_or(0),
            username: row.get(4)?,
            password: row.get(5)?,
        };
        server.password = cipher::open(key, &server.binding(), &server.password).unwrap_or_else(|err| {
            eprintln!("database: the password of {} cannot be opened: {err}", server.address);
            String::new()
        });
        Ok(server)
    })?;
    rows.collect()
}

/// `servers` written as they are: their passwords sealed already.
fn write_servers(connection: &Connection, servers: &[ServerRecord]) -> rusqlite::Result<()> {
    connection.execute_batch("BEGIN; DELETE FROM servers;")?;
    let written: rusqlite::Result<()> = servers.iter().enumerate().try_for_each(|(position, server)| {
        connection
            .execute(
                "INSERT INTO servers (position, name, protocol, address, port, username, password) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![position as i64, server.name, server.protocol, server.address, i64::from(server.port), server.username, server.password],
            )
            .map(|_| ())
    });
    match written {
        Ok(()) => connection.execute_batch("COMMIT;"),
        Err(err) => {
            let _ = connection.execute_batch("ROLLBACK;");
            Err(err)
        }
    }
}

// ---- the SSH servers' keys ---------------------------------------------------------------------

/// The key an SSH server is trusted by (src/downloads/sftp.rs), and since when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshHost {
    /// The public key, in OpenSSH's one-line form.
    pub key: String,
    /// When the user trusted it, in seconds since the epoch; 0 for a key that FinchGram 0.6.2 or
    /// before took on its own, without asking.
    pub first_seen: i64,
}

/// The key the SSH server at `host`:`port` is trusted by; none for a server not trusted yet.
pub fn ssh_host(host: &str, port: u16) -> Option<SshHost> {
    with(|connection| read_ssh_host(connection, host, port)).unwrap_or_else(|err| {
        eprintln!("database: cannot read a server's key: {err}");
        None
    })
}

/// Trust `key` as the server's, from now, in place of what was kept before; the date stays when
/// the key is the one kept already.
pub fn trust_ssh_host(host: &str, port: u16, key: &str) -> Result<(), String> {
    with(|connection| write_ssh_host(connection, host, port, key, chrono::Local::now().timestamp()))
}

/// The server's key forgotten: the next connection asks the user about the key it shows.
pub fn forget_ssh_host(host: &str, port: u16) -> Result<(), String> {
    with(|connection| delete_ssh_host(connection, host, port))
}

fn read_ssh_host(connection: &Connection, host: &str, port: u16) -> rusqlite::Result<Option<SshHost>> {
    connection
        .query_row("SELECT key, first_seen FROM ssh_hosts WHERE host = ?1 AND port = ?2", params![host, i64::from(port)], |row| {
            Ok(SshHost { key: row.get(0)?, first_seen: row.get(1)? })
        })
        .optional()
}

fn write_ssh_host(connection: &Connection, host: &str, port: u16, key: &str, now: i64) -> rusqlite::Result<()> {
    connection
        .execute(
            "INSERT INTO ssh_hosts (host, port, key, first_seen) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(host, port) DO UPDATE SET
                 first_seen = CASE WHEN ssh_hosts.key = excluded.key THEN ssh_hosts.first_seen ELSE excluded.first_seen END,
                 key = excluded.key",
            params![host, i64::from(port), key, now],
        )
        .map(|_| ())
}

fn delete_ssh_host(connection: &Connection, host: &str, port: u16) -> rusqlite::Result<()> {
    connection.execute("DELETE FROM ssh_hosts WHERE host = ?1 AND port = ?2", params![host, i64::from(port)]).map(|_| ())
}

// ---- the programs at AI access -----------------------------------------------------------------

/// The programs that have connected to the MCP server, in the order they first did; a row that
/// is not FinchGram's as it wrote it is left out.
pub fn mcp_apps() -> Vec<McpApp> {
    let Some(sealing) = sealing_key() else { return Vec::new() };
    match with(|connection| read_mcp_apps(connection, &sealing)) {
        Ok(apps) => apps,
        Err(err) => {
            eprintln!("database: cannot read the programs: {err}");
            Vec::new()
        }
    }
}

/// The program remembered by `key`, if it has connected before; none for a row that is not
/// FinchGram's as it wrote it, so the program is asked about as if it were new.
pub fn mcp_app(key: &str) -> Option<McpApp> {
    let sealing = sealing_key()?;
    with(|connection| read_mcp_app(connection, &sealing, key)).unwrap_or_else(|err| {
        eprintln!("database: cannot read a program: {err}");
        None
    })
}

/// Keep `app`, in place of what was kept under its key.
pub fn save_mcp_app(app: &McpApp) -> Result<(), String> {
    let sealing = sealing_key().ok_or("the database is not open")?;
    with(|connection| write_mcp_app(connection, &sealing, app))
}

/// Forget the program remembered by `key`; nothing happens when there is none.
pub fn remove_mcp_app(key: &str) -> Result<(), String> {
    with(|connection| connection.execute("DELETE FROM mcp_apps WHERE key = ?1", params![key]).map(|_| ()))
}

/// Forget every program (another Telegram account logged in: src/mcp/access.rs).
pub fn clear_mcp_apps() -> Result<(), String> {
    with(|connection| connection.execute("DELETE FROM mcp_apps", []).map(|_| ()))
}

fn read_mcp_apps(connection: &Connection, sealing: &[u8; 32]) -> rusqlite::Result<Vec<McpApp>> {
    let mut statement = connection.prepare("SELECT key, path, name, allowed, last_used, seal FROM mcp_apps ORDER BY rowid")?;
    let rows: Vec<(McpApp, String)> = statement.query_map([], mcp_app_row)?.collect::<rusqlite::Result<_>>()?;
    Ok(rows.into_iter().filter_map(|row| believed(sealing, row)).collect())
}

fn read_mcp_app(connection: &Connection, sealing: &[u8; 32], key: &str) -> rusqlite::Result<Option<McpApp>> {
    let row = connection.query_row("SELECT key, path, name, allowed, last_used, seal FROM mcp_apps WHERE key = ?1", params![key], mcp_app_row).optional()?;
    Ok(row.and_then(|row| believed(sealing, row)))
}

fn mcp_app_row(row: &rusqlite::Row) -> rusqlite::Result<(McpApp, String)> {
    Ok((McpApp { key: row.get(0)?, path: row.get(1)?, name: row.get(2)?, allowed: row.get::<_, i64>(3)? != 0, last_used: row.get(4)? }, row.get(5)?))
}

/// The row when it is FinchGram's as it wrote it: its tag is the row's under the key only this app
/// has. One written by anything else, or with its answer turned since, is not.
fn believed(sealing: &[u8; 32], (app, seal): (McpApp, String)) -> Option<McpApp> {
    if cipher::tag_matches(sealing, &app.sealed_text(), &seal) {
        return Some(app);
    }
    eprintln!("database: the row of the program {} was not written by FinchGram, or was changed since: not believed", app.key);
    None
}

fn write_mcp_app(connection: &Connection, sealing: &[u8; 32], app: &McpApp) -> rusqlite::Result<()> {
    let seal = cipher::tag(sealing, &app.sealed_text());
    connection
        .execute(
            "INSERT INTO mcp_apps (key, path, name, allowed, last_used, seal) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(key) DO UPDATE SET path = excluded.path, name = excluded.name, allowed = excluded.allowed, last_used = excluded.last_used, seal = excluded.seal",
            params![app.key, app.path, app.name, i64::from(app.allowed), app.last_used, seal],
        )
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh folder for a test's file and its backups.
    fn scratch(name: &str) -> (PathBuf, PathBuf) {
        let folder = std::env::temp_dir().join(format!("finchgram-database-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        (folder.join("finchgram.db"), folder.join("backup"))
    }

    fn a_key(byte: u8) -> [u8; 32] {
        [byte; 32]
    }

    fn a_server(name: &str, password: &str) -> ServerRecord {
        ServerRecord { name: name.into(), protocol: "webdav".into(), address: "https://nas.home/dav".into(), port: 5006, username: "me".into(), password: password.into() }
    }

    fn sealed_copies(servers: &[ServerRecord], key: &[u8; 32]) -> Vec<ServerRecord> {
        servers.iter().map(|server| ServerRecord { password: cipher::seal(key, &server.binding(), &server.password).expect("sealed"), ..server.clone() }).collect()
    }

    /// A file as 0.6.0 made it: layout 1, a server with its password sealed to nothing, and a
    /// program allowed, its row without a tag.
    fn layout_one(path: &Path, key: &[u8; 32]) {
        std::fs::create_dir_all(path.parent().expect("the folder")).expect("mkdir");
        let connection = Connection::open(path).expect("a file");
        connection.execute_batch(TABLES_OF_LAYOUT_1).expect("the tables of layout 1");
        connection
            .execute(
                "INSERT INTO servers (position, name, protocol, address, port, username, password) VALUES (0, 'Home NAS', 'webdav', 'https://nas.home/dav', 5006, 'me', ?1)",
                params![cipher::seal_unbound(key, "secret")],
            )
            .expect("a server");
        connection
            .execute("INSERT INTO mcp_apps (key, path, name, allowed, last_used) VALUES ('Q6L2SF6YDW/com.anthropic.claude-code', '/x', 'claude', 1, 10)", [])
            .expect("a program");
        connection.execute("INSERT INTO ssh_hosts (host, port, key) VALUES ('sb-ams.example.net', 22, 'ssh-ed25519 AAAA1')", []).expect("a key");
        connection.execute_batch("PRAGMA user_version = 1;").expect("stamped");
    }

    #[test]
    fn a_new_file_gets_the_newest_layout_and_no_backup() {
        let (path, backup) = scratch("new");
        let connection = open_at(&path, &a_key(1), &backup).expect("a new database");
        let version: i32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0)).expect("version");
        assert_eq!(version, LAYOUT);
        assert_eq!(read_servers(&connection, &a_key(1)).expect("read"), vec![]);
        assert_eq!(read_mcp_apps(&connection, &a_key(1)).expect("read"), vec![]);
        assert!(!backup.exists());
        drop(connection);
        // Opened again, it is the same layout, and still nothing is copied.
        let connection = open_at(&path, &a_key(1), &backup).expect("opened again");
        assert_eq!(connection.query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0)).expect("version"), LAYOUT);
        assert!(!backup.exists());
        let _ = std::fs::remove_dir_all(path.parent().expect("the folder"));
    }

    #[test]
    fn servers_are_kept_in_order_with_their_passwords_sealed() {
        let (path, backup) = scratch("servers");
        let key = a_key(2);
        let servers = vec![a_server("Home NAS", "secret"), ServerRecord { protocol: "sftp".into(), address: "sb-ams.example.net".into(), port: 0, ..a_server("Seedbox", "") }];
        {
            let connection = open_at(&path, &key, &backup).expect("a new database");
            write_servers(&connection, &sealed_copies(&servers, &key)).expect("write");
            write_servers(&connection, &sealed_copies(&servers[1..], &key)).expect("write again");
            assert_eq!(read_servers(&connection, &key).expect("read"), servers[1..]);
            write_servers(&connection, &sealed_copies(&servers, &key)).expect("write all");
            assert_eq!(read_servers(&connection, &key).expect("read"), servers);
            // The file is a plain database, with the password not in it as it is.
            connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").expect("checkpoint");
        }
        let bytes = std::fs::read(&path).expect("the file's bytes");
        assert!(bytes.starts_with(SQLITE_HEADER));
        assert!(bytes.windows(8).any(|window| window == b"Home NAS"));
        assert!(!bytes.windows(6).any(|window| window == b"secret"));
        // Another key opens the file (it is plain) but not the password.
        let connection = open_at(&path, &a_key(3), &backup).expect("opened");
        let read = read_servers(&connection, &a_key(3)).expect("read");
        assert_eq!(read[0].name, "Home NAS");
        assert_eq!(read[0].password, "");
        assert_eq!(read[1].password, "");
        drop(connection);
        // The sealed text moved to a row naming another server (what a program writing the file
        // could do, to have the app send it the password) does not open there, with the right key.
        let connection = open_at(&path, &key, &backup).expect("opened");
        connection.execute("UPDATE servers SET address = 'https://evil.example/dav' WHERE name = 'Home NAS'", []).expect("moved");
        let read = read_servers(&connection, &key).expect("read");
        assert_eq!((read[0].address.as_str(), read[0].password.as_str()), ("https://evil.example/dav", ""));
        for column in ["port = 5007", "username = 'you'", "protocol = 'ftp'"] {
            connection
                .execute("UPDATE servers SET address = 'https://nas.home/dav', port = 5006, username = 'me', protocol = 'webdav' WHERE name = 'Home NAS'", [])
                .expect("back");
            assert_eq!(read_servers(&connection, &key).expect("read")[0].password, "secret");
            connection.execute(&format!("UPDATE servers SET {column} WHERE name = 'Home NAS'"), []).expect("changed");
            assert_eq!(read_servers(&connection, &key).expect("read")[0].password, "", "{column}");
        }
        let _ = std::fs::remove_dir_all(path.parent().expect("the folder"));
    }

    #[test]
    fn a_password_sealed_twice_looks_different_and_opens_the_same() {
        let key = a_key(4);
        let server = a_server("Home NAS", "correct horse");
        let one = cipher::seal(&key, &server.binding(), "correct horse").expect("sealed");
        let two = cipher::seal(&key, &server.binding(), "correct horse").expect("sealed again");
        assert_ne!(one, two);
        assert!(one.starts_with("v2:"));
        assert_eq!(cipher::open(&key, &server.binding(), &one).expect("opened"), "correct horse");
        assert_eq!(cipher::open(&key, &server.binding(), &two).expect("opened"), "correct horse");
        assert_eq!(cipher::seal(&key, &server.binding(), "").expect("sealed"), "");
        assert_eq!(cipher::open(&key, &server.binding(), "").expect("opened"), "");
        assert!(cipher::open(&a_key(5), &server.binding(), &one).is_err());
        let other = ServerRecord { address: "https://other.home/dav".into(), ..server.clone() };
        assert!(cipher::open(&key, &other.binding(), &one).is_err());
        assert!(cipher::open(&key, &server.binding(), "v2:not base64").is_err());
        assert!(cipher::open(&key, &server.binding(), "plain").is_err());
        // What layout 1 wrote opens for the upgrade step only.
        let old = cipher::seal_unbound(&key, "correct horse");
        assert!(old.starts_with("v1:"));
        assert!(cipher::open(&key, &server.binding(), &old).is_err());
        assert_eq!(cipher::open_unbound(&key, &old).expect("opened"), "correct horse");
        assert!(cipher::open_unbound(&a_key(5), &old).is_err());
        assert!(cipher::open_unbound(&key, &one).is_err());
    }

    #[test]
    fn a_rows_tag_holds_to_its_text_and_its_key() {
        let key = a_key(11);
        let tag = cipher::tag(&key, "mcp_app\0Q6L2SF6YDW/com.anthropic.claude-code\x001");
        assert!(cipher::tag_matches(&key, "mcp_app\0Q6L2SF6YDW/com.anthropic.claude-code\x001", &tag));
        assert!(!cipher::tag_matches(&key, "mcp_app\0Q6L2SF6YDW/com.anthropic.claude-code\x000", &tag));
        assert!(!cipher::tag_matches(&a_key(12), "mcp_app\0Q6L2SF6YDW/com.anthropic.claude-code\x001", &tag));
        assert!(!cipher::tag_matches(&key, "mcp_app\0Q6L2SF6YDW/com.anthropic.claude-code\x001", ""));
        assert!(!cipher::tag_matches(&key, "mcp_app\0Q6L2SF6YDW/com.anthropic.claude-code\x001", "not base64!"));
        assert_eq!(tag, cipher::tag(&key, "mcp_app\0Q6L2SF6YDW/com.anthropic.claude-code\x001"));
    }

    #[test]
    fn the_encrypted_file_of_an_older_finchgram_is_put_aside_and_a_new_one_made() {
        let (path, backup) = scratch("encrypted");
        std::fs::create_dir_all(path.parent().expect("the folder")).expect("mkdir");
        let scrambled: Vec<u8> = (0..4096u32).map(|i| (i * 7 % 251) as u8 ^ 0x5a).collect();
        std::fs::write(&path, &scrambled).expect("an old file");
        std::fs::write(with_suffix(&path, "-wal"), b"journal").expect("its journal");
        let connection = open_at(&path, &a_key(6), &backup).expect("a new database in its place");
        assert_eq!(read_servers(&connection, &a_key(6)).expect("read"), vec![]);
        let moved: Vec<PathBuf> = std::fs::read_dir(&backup).expect("the backup folder").flatten().map(|entry| entry.path()).collect();
        let db = moved.iter().find(|p| p.to_string_lossy().ends_with(".db")).expect("the old file, moved");
        assert!(db.file_name().expect("name").to_string_lossy().starts_with("finchgram-encrypted-before-0.6.0-"));
        assert_eq!(std::fs::read(db).expect("its bytes"), scrambled);
        assert!(moved.iter().any(|p| p.to_string_lossy().ends_with(".db-wal")));
        let _ = std::fs::remove_dir_all(path.parent().expect("the folder"));
    }

    #[test]
    fn a_file_of_a_newer_finchgram_is_refused_and_left_alone() {
        let (path, backup) = scratch("newer");
        {
            let connection = open_at(&path, &a_key(7), &backup).expect("a new database");
            connection.execute_batch(&format!("PRAGMA user_version = {};", LAYOUT + 1)).expect("stamped");
        }
        let err = open_at(&path, &a_key(7), &backup).expect_err("refused");
        assert!(err.contains("newer FinchGram"), "{err}");
        assert!(!backup.exists());
        let _ = std::fs::remove_dir_all(path.parent().expect("the folder"));
    }

    #[test]
    fn an_older_layout_is_copied_aside_and_brought_up() {
        let (path, backup) = scratch("older");
        let key = a_key(8);
        layout_one(&path, &key);
        let connection = open_at(&path, &key, &backup).expect("brought up");
        assert_eq!(connection.query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0)).expect("version"), LAYOUT);
        // The password is sealed to its server now, and opens.
        let servers = read_servers(&connection, &key).expect("read");
        assert_eq!((servers[0].name.as_str(), servers[0].password.as_str()), ("Home NAS", "secret"));
        let stored: String = connection.query_row("SELECT password FROM servers", [], |row| row.get(0)).expect("the column");
        assert!(stored.starts_with("v2:"), "{stored}");
        // The programs of before are not carried over: nothing vouches for their rows.
        assert_eq!(read_mcp_apps(&connection, &key).expect("read"), vec![]);
        assert_eq!(connection.query_row("SELECT count(*) FROM mcp_apps", [], |row| row.get::<_, i64>(0)).expect("count"), 0);
        // A server's key taken on by 0.6.2 stays trusted, with no day to it.
        assert_eq!(read_ssh_host(&connection, "sb-ams.example.net", 22).expect("read"), Some(SshHost { key: "ssh-ed25519 AAAA1".into(), first_seen: 0 }));
        // The file as it was is in the backup folder, the password sealed as before.
        let copies: Vec<PathBuf> = std::fs::read_dir(&backup).expect("the backup folder").flatten().map(|entry| entry.path()).collect();
        assert_eq!(copies.len(), 1);
        assert!(copies[0].file_name().expect("name").to_string_lossy().starts_with("finchgram-layout-1-"));
        let copy = Connection::open(&copies[0]).expect("the copy");
        let kept: String = copy.query_row("SELECT password FROM servers", [], |row| row.get(0)).expect("the column");
        assert!(kept.starts_with("v1:"), "{kept}");
        assert_eq!(copy.query_row("SELECT count(*) FROM mcp_apps", [], |row| row.get::<_, i64>(0)).expect("count"), 1);
        drop(copy);
        drop(connection);
        // Opened again: layout 2 already, nothing more is copied.
        let connection = open_at(&path, &key, &backup).expect("opened again");
        assert_eq!(read_servers(&connection, &key).expect("read")[0].password, "secret");
        assert_eq!(std::fs::read_dir(&backup).expect("the backup folder").count(), 1);
        let _ = std::fs::remove_dir_all(path.parent().expect("the folder"));
    }

    #[test]
    fn a_password_of_layout_one_under_a_key_that_is_gone_is_left_empty() {
        let (path, backup) = scratch("older-key");
        layout_one(&path, &a_key(13));
        let connection = open_at(&path, &a_key(14), &backup).expect("brought up");
        let servers = read_servers(&connection, &a_key(14)).expect("read");
        assert_eq!((servers[0].name.as_str(), servers[0].password.as_str()), ("Home NAS", ""));
        let _ = std::fs::remove_dir_all(path.parent().expect("the folder"));
    }

    #[test]
    fn programs_are_kept_by_their_key_and_forgotten_one_by_one_or_all_at_once() {
        let (path, backup) = scratch("apps");
        let key = a_key(9);
        let connection = open_at(&path, &key, &backup).expect("a new database");
        assert_eq!(read_mcp_apps(&connection, &key).expect("read"), vec![]);
        let claude = McpApp {
            key: "Q6L2SF6YDW/com.anthropic.claudefordesktop".into(),
            path: "/Applications/Claude.app".into(),
            name: "Claude".into(),
            allowed: true,
            last_used: 10,
        };
        let code = McpApp {
            key: "Q6L2SF6YDW/com.anthropic.claude-code".into(),
            path: "/Users/x/.local/share/claude/versions/2.1.306".into(),
            name: "claude".into(),
            allowed: false,
            last_used: 0,
        };
        write_mcp_app(&connection, &key, &claude).expect("write");
        write_mcp_app(&connection, &key, &code).expect("write");
        assert_eq!(read_mcp_apps(&connection, &key).expect("read"), vec![claude.clone(), code.clone()]);
        // Again under the same key, from another copy of the program: the one record, brought up
        // to date, where it is now included.
        let elsewhere = "/Users/x/.vscode/extensions/anthropic.claude-code-2.1.305-darwin-arm64/resources/native-binary/claude";
        let used = McpApp { path: elsewhere.into(), last_used: 20, ..code.clone() };
        write_mcp_app(&connection, &key, &used).expect("write again");
        assert_eq!(read_mcp_app(&connection, &key, &code.key).expect("read one"), Some(used));
        assert_eq!(read_mcp_app(&connection, &key, "/nowhere").expect("read none"), None);
        connection.execute("DELETE FROM mcp_apps WHERE key = ?1", params![code.key]).expect("remove");
        assert_eq!(read_mcp_apps(&connection, &key).expect("read").len(), 1);
        connection.execute("DELETE FROM mcp_apps", []).expect("clear");
        assert_eq!(read_mcp_apps(&connection, &key).expect("read"), vec![]);
        drop(connection);
        let _ = std::fs::remove_dir_all(path.parent().expect("the folder"));
    }

    #[test]
    fn a_programs_row_written_by_anything_else_or_turned_since_is_not_believed() {
        let (path, backup) = scratch("apps-forged");
        let key = a_key(15);
        let connection = open_at(&path, &key, &backup).expect("a new database");
        let code = McpApp { key: "Q6L2SF6YDW/com.anthropic.claude-code".into(), path: "/x".into(), name: "claude".into(), allowed: false, last_used: 0 };
        write_mcp_app(&connection, &key, &code).expect("write");
        // A row another program put into the file, allowing itself: not FinchGram's.
        connection
            .execute("INSERT INTO mcp_apps (key, path, name, allowed, last_used, seal) VALUES ('/Users/x/evil', '/Users/x/evil', 'evil', 1, 0, '')", [])
            .expect("a forged row");
        connection
            .execute("INSERT INTO mcp_apps (key, path, name, allowed, last_used, seal) VALUES ('/Users/x/evil2', '/Users/x/evil2', 'evil', 1, 0, ?1)", params![cipher::tag(&a_key(16), "mcp_app\0/Users/x/evil2\x001")])
            .expect("a forged row under a guessed key");
        assert_eq!(read_mcp_app(&connection, &key, "/Users/x/evil").expect("read"), None);
        assert_eq!(read_mcp_app(&connection, &key, "/Users/x/evil2").expect("read"), None);
        assert_eq!(read_mcp_apps(&connection, &key).expect("read"), vec![code.clone()]);
        // FinchGram's own row with its answer turned behind its back: not believed either, so the
        // program is asked about again.
        connection.execute("UPDATE mcp_apps SET allowed = 1 WHERE key = ?1", params![code.key]).expect("turned");
        assert_eq!(read_mcp_app(&connection, &key, &code.key).expect("read"), None);
        assert_eq!(read_mcp_apps(&connection, &key).expect("read"), vec![]);
        // Where it is, its name and its last use are only shown: a change there is no forgery.
        write_mcp_app(&connection, &key, &McpApp { allowed: true, ..code.clone() }).expect("allowed");
        connection.execute("UPDATE mcp_apps SET path = '/elsewhere', last_used = 99 WHERE key = ?1", params![code.key]).expect("moved");
        assert_eq!(read_mcp_app(&connection, &key, &code.key).expect("read").map(|app| (app.allowed, app.path)), Some((true, "/elsewhere".to_string())));
        // The row's tag from one file does not carry its answer into another: the key differs.
        assert_eq!(read_mcp_app(&connection, &a_key(17), &code.key).expect("read"), None);
        drop(connection);
        let _ = std::fs::remove_dir_all(path.parent().expect("the folder"));
    }

    #[test]
    fn ssh_servers_keys_are_kept_by_host_and_port_with_the_day_they_were_trusted() {
        let (path, backup) = scratch("ssh");
        let connection = open_at(&path, &a_key(10), &backup).expect("a new database");
        let host = |key: &str, first_seen: i64| Some(SshHost { key: key.into(), first_seen });
        assert_eq!(read_ssh_host(&connection, "sb-ams.example.net", 22).expect("read"), None);
        write_ssh_host(&connection, "sb-ams.example.net", 22, "ssh-ed25519 AAAA1", 1_000).expect("trusted");
        write_ssh_host(&connection, "sb-ams.example.net", 2222, "ssh-ed25519 AAAA2", 2_000).expect("trusted");
        assert_eq!(read_ssh_host(&connection, "sb-ams.example.net", 22).expect("read"), host("ssh-ed25519 AAAA1", 1_000));
        assert_eq!(read_ssh_host(&connection, "sb-ams.example.net", 2222).expect("read"), host("ssh-ed25519 AAAA2", 2_000));
        // Trusted again, the same key: the day stays. Another key: the day is this one.
        write_ssh_host(&connection, "sb-ams.example.net", 22, "ssh-ed25519 AAAA1", 3_000).expect("trusted again");
        assert_eq!(read_ssh_host(&connection, "sb-ams.example.net", 22).expect("read"), host("ssh-ed25519 AAAA1", 1_000));
        write_ssh_host(&connection, "sb-ams.example.net", 22, "ssh-ed25519 BBBB", 4_000).expect("a new key");
        assert_eq!(read_ssh_host(&connection, "sb-ams.example.net", 22).expect("read"), host("ssh-ed25519 BBBB", 4_000));
        // Forgotten: nothing is known, and forgetting twice is nothing.
        delete_ssh_host(&connection, "sb-ams.example.net", 22).expect("forgotten");
        delete_ssh_host(&connection, "sb-ams.example.net", 22).expect("forgotten again");
        assert_eq!(read_ssh_host(&connection, "sb-ams.example.net", 22).expect("read"), None);
        assert_eq!(read_ssh_host(&connection, "sb-ams.example.net", 2222).expect("read"), host("ssh-ed25519 AAAA2", 2_000));
        drop(connection);
        let _ = std::fs::remove_dir_all(path.parent().expect("the folder"));
    }

    #[test]
    fn a_key_must_be_thirty_two_bytes_of_base64() {
        assert!(decode_key("not base64!").is_err());
        assert!(decode_key(&base64::engine::general_purpose::STANDARD.encode([7u8; 16])).is_err());
        assert_eq!(decode_key(&base64::engine::general_purpose::STANDARD.encode([7u8; 32])).expect("a key"), [7u8; 32]);
    }
}
