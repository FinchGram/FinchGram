//! The app's own database: a plain SQLite file, `finchgram.db` next to settings.toml, that any
//! SQLite tool opens, since the user's data is the user's (the maintainer's word, 2026-10-24).
//! It holds what FinchGram keeps that is neither a preference (settings.toml is for those, and
//! only those) nor Telegram's (that is TDLib's database): the servers of the downloads page with
//! their passwords, the programs the user has allowed or turned away at AI access, and the keys
//! of the SSH servers met. Only the password column is sealed (AES-256-GCM, `cipher` below) under
//! a random key made the first time and kept in the Keychain under FinchGram's name
//! (platform::KEYCHAIN_SERVICE, the account "database"), where only this app reads it; every
//! other column is in the clear. TDLib's database key is kept the same way (src/telegram/mod.rs).
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
pub const LAYOUT: i32 = 1;

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
    let _ = key;
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
        upgrade(&connection, 0)?;
        return Ok(connection);
    }
    if version > LAYOUT {
        return Err(format!("{} was made by a newer FinchGram (layout {version}; this one knows {LAYOUT}): update FinchGram to use it", path.display()));
    }
    if version < LAYOUT {
        copy_aside(&connection, path, backup, &format!("layout-{version}"))?;
        upgrade(&connection, version)?;
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
/// by stamping the version.
fn upgrade(connection: &Connection, from: i32) -> Result<(), String> {
    for to in (from + 1)..=LAYOUT {
        let sql = match to {
            1 => {
                "CREATE TABLE IF NOT EXISTS servers (
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
                 );"
            }
            _ => return Err(format!("no step to layout {to}")),
        };
        connection
            .execute_batch(&format!("BEGIN; {sql} PRAGMA user_version = {to}; COMMIT;"))
            .map_err(|err| format!("cannot bring the database to layout {to}: {err}"))?;
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

/// The password column: AES-256-GCM under the Keychain's key, a fresh nonce each time, written
/// as base64 of nonce + ciphertext after "v1:". "" stays "". A tool opening the file sees the
/// column as that text; the app shows the password in the server's form.
mod cipher {
    use base64::Engine as _;
    use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
    use ring::rand::{SecureRandom as _, SystemRandom};

    const PREFIX: &str = "v1:";

    pub fn seal(key: &[u8; 32], password: &str) -> Result<String, String> {
        if password.is_empty() {
            return Ok(String::new());
        }
        let sealer = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).map_err(|_| "the sealing key is not one")?);
        let mut nonce = [0u8; NONCE_LEN];
        SystemRandom::new().fill(&mut nonce).map_err(|_| "the system gave no random bytes for a nonce")?;
        let mut data = password.as_bytes().to_vec();
        sealer.seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::empty(), &mut data).map_err(|_| "sealing the password failed")?;
        let mut whole = nonce.to_vec();
        whole.extend_from_slice(&data);
        Ok(format!("{PREFIX}{}", base64::engine::general_purpose::STANDARD.encode(whole)))
    }

    pub fn open(key: &[u8; 32], sealed: &str) -> Result<String, String> {
        if sealed.is_empty() {
            return Ok(String::new());
        }
        let encoded = sealed.strip_prefix(PREFIX).ok_or("the sealed password does not start with v1:")?;
        let mut whole = base64::engine::general_purpose::STANDARD.decode(encoded).map_err(|err| format!("the sealed password is not base64: {err}"))?;
        if whole.len() < NONCE_LEN {
            return Err("the sealed password is too short".into());
        }
        let data = whole.split_off(NONCE_LEN);
        let nonce: [u8; NONCE_LEN] = whole.try_into().map_err(|_| "the nonce is not one")?;
        let opener = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).map_err(|_| "the sealing key is not one")?);
        let mut data = data;
        let plain = opener.open_in_place(Nonce::assume_unique_for_key(nonce), Aad::empty(), &mut data).map_err(|_| "the password was sealed with another key")?;
        String::from_utf8(plain.to_vec()).map_err(|_| "the password is not text".into())
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
        sealed.push(ServerRecord { password: cipher::seal(&key, &server.password)?, ..server.clone() });
    }
    with(|connection| write_servers(connection, &sealed))
}

fn read_servers(connection: &Connection, key: &[u8; 32]) -> rusqlite::Result<Vec<ServerRecord>> {
    let mut statement = connection.prepare("SELECT name, protocol, address, port, username, password FROM servers ORDER BY position, id")?;
    let rows = statement.query_map([], |row| {
        let sealed: String = row.get(5)?;
        let password = cipher::open(key, &sealed).unwrap_or_else(|err| {
            eprintln!("database: a server's password cannot be opened: {err}");
            String::new()
        });
        Ok(ServerRecord {
            name: row.get(0)?,
            protocol: row.get(1)?,
            address: row.get(2)?,
            port: u16::try_from(row.get::<_, i64>(3)?).unwrap_or(0),
            username: row.get(4)?,
            password,
        })
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

/// The public key an SSH server showed when it was first met, in OpenSSH's one-line form; none
/// for a server not met yet.
pub fn ssh_host_key(host: &str, port: u16) -> Option<String> {
    with(|connection| read_ssh_host_key(connection, host, port)).unwrap_or_else(|err| {
        eprintln!("database: cannot read a server's key: {err}");
        None
    })
}

/// Remember `key` as the server's, in place of what was kept before.
pub fn save_ssh_host_key(host: &str, port: u16, key: &str) -> Result<(), String> {
    with(|connection| {
        connection
            .execute(
                "INSERT INTO ssh_hosts (host, port, key) VALUES (?1, ?2, ?3) ON CONFLICT(host, port) DO UPDATE SET key = excluded.key",
                params![host, i64::from(port), key],
            )
            .map(|_| ())
    })
}

fn read_ssh_host_key(connection: &Connection, host: &str, port: u16) -> rusqlite::Result<Option<String>> {
    connection.query_row("SELECT key FROM ssh_hosts WHERE host = ?1 AND port = ?2", params![host, i64::from(port)], |row| row.get(0)).optional()
}

// ---- the programs at AI access -----------------------------------------------------------------

/// The programs that have connected to the MCP server, in the order they first did.
pub fn mcp_apps() -> Vec<McpApp> {
    match with(read_mcp_apps) {
        Ok(apps) => apps,
        Err(err) => {
            eprintln!("database: cannot read the programs: {err}");
            Vec::new()
        }
    }
}

/// The program remembered by `key`, if it has connected before.
pub fn mcp_app(key: &str) -> Option<McpApp> {
    with(|connection| read_mcp_app(connection, key)).unwrap_or_else(|err| {
        eprintln!("database: cannot read a program: {err}");
        None
    })
}

/// Keep `app`, in place of what was kept under its key.
pub fn save_mcp_app(app: &McpApp) -> Result<(), String> {
    with(|connection| write_mcp_app(connection, app))
}

/// Forget the program remembered by `key`; nothing happens when there is none.
pub fn remove_mcp_app(key: &str) -> Result<(), String> {
    with(|connection| connection.execute("DELETE FROM mcp_apps WHERE key = ?1", params![key]).map(|_| ()))
}

/// Forget every program (another Telegram account logged in: src/mcp/access.rs).
pub fn clear_mcp_apps() -> Result<(), String> {
    with(|connection| connection.execute("DELETE FROM mcp_apps", []).map(|_| ()))
}

fn read_mcp_apps(connection: &Connection) -> rusqlite::Result<Vec<McpApp>> {
    let mut statement = connection.prepare("SELECT key, path, name, allowed, last_used FROM mcp_apps ORDER BY rowid")?;
    let rows = statement.query_map([], mcp_app_row)?;
    rows.collect()
}

fn read_mcp_app(connection: &Connection, key: &str) -> rusqlite::Result<Option<McpApp>> {
    connection.query_row("SELECT key, path, name, allowed, last_used FROM mcp_apps WHERE key = ?1", params![key], mcp_app_row).optional()
}

fn mcp_app_row(row: &rusqlite::Row) -> rusqlite::Result<McpApp> {
    Ok(McpApp { key: row.get(0)?, path: row.get(1)?, name: row.get(2)?, allowed: row.get::<_, i64>(3)? != 0, last_used: row.get(4)? })
}

fn write_mcp_app(connection: &Connection, app: &McpApp) -> rusqlite::Result<()> {
    connection
        .execute(
            "INSERT INTO mcp_apps (key, path, name, allowed, last_used) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(key) DO UPDATE SET path = excluded.path, name = excluded.name, allowed = excluded.allowed, last_used = excluded.last_used",
            params![app.key, app.path, app.name, i64::from(app.allowed), app.last_used],
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
        servers.iter().map(|server| ServerRecord { password: cipher::seal(key, &server.password).expect("sealed"), ..server.clone() }).collect()
    }

    fn opened_copies(servers: &[ServerRecord], key: &[u8; 32]) -> Vec<ServerRecord> {
        servers.iter().map(|server| ServerRecord { password: cipher::open(key, &server.password).expect("opened"), ..server.clone() }).collect()
    }

    #[test]
    fn a_new_file_gets_the_newest_layout_and_no_backup() {
        let (path, backup) = scratch("new");
        let connection = open_at(&path, &a_key(1), &backup).expect("a new database");
        let version: i32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0)).expect("version");
        assert_eq!(version, LAYOUT);
        assert_eq!(read_servers(&connection, &a_key(1)).expect("read"), vec![]);
        assert_eq!(read_mcp_apps(&connection).expect("read"), vec![]);
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
        let _ = std::fs::remove_dir_all(path.parent().expect("the folder"));
    }

    #[test]
    fn a_password_sealed_twice_looks_different_and_opens_the_same() {
        let key = a_key(4);
        let one = cipher::seal(&key, "correct horse").expect("sealed");
        let two = cipher::seal(&key, "correct horse").expect("sealed again");
        assert_ne!(one, two);
        assert!(one.starts_with("v1:"));
        assert_eq!(cipher::open(&key, &one).expect("opened"), "correct horse");
        assert_eq!(cipher::open(&key, &two).expect("opened"), "correct horse");
        assert_eq!(cipher::seal(&key, "").expect("sealed"), "");
        assert_eq!(cipher::open(&key, "").expect("opened"), "");
        assert!(cipher::open(&a_key(5), &one).is_err());
        assert!(cipher::open(&key, "v1:not base64").is_err());
        assert!(cipher::open(&key, "plain").is_err());
        let _ = opened_copies(&[], &key);
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
        {
            // A file with tables but stamped before layout 1: the oldest a plain file can be.
            let connection = open_at(&path, &a_key(8), &backup).expect("a new database");
            connection.execute_batch("PRAGMA user_version = 0;").expect("stamped back");
            write_servers(&connection, &sealed_copies(&[a_server("Home NAS", "pw")], &a_key(8))).expect("write");
        }
        let connection = open_at(&path, &a_key(8), &backup).expect("brought up");
        assert_eq!(connection.query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0)).expect("version"), LAYOUT);
        assert_eq!(read_servers(&connection, &a_key(8)).expect("read")[0].name, "Home NAS");
        let copies: Vec<PathBuf> = std::fs::read_dir(&backup).expect("the backup folder").flatten().map(|entry| entry.path()).collect();
        assert_eq!(copies.len(), 1);
        assert!(copies[0].file_name().expect("name").to_string_lossy().starts_with("finchgram-layout-0-"));
        let _ = std::fs::remove_dir_all(path.parent().expect("the folder"));
    }

    #[test]
    fn programs_are_kept_by_their_key_and_forgotten_one_by_one_or_all_at_once() {
        let (path, backup) = scratch("apps");
        let connection = open_at(&path, &a_key(9), &backup).expect("a new database");
        assert_eq!(read_mcp_apps(&connection).expect("read"), vec![]);
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
        write_mcp_app(&connection, &claude).expect("write");
        write_mcp_app(&connection, &code).expect("write");
        assert_eq!(read_mcp_apps(&connection).expect("read"), vec![claude.clone(), code.clone()]);
        // Again under the same key, from another copy of the program: the one record, brought up
        // to date, where it is now included.
        let elsewhere = "/Users/x/.vscode/extensions/anthropic.claude-code-2.1.305-darwin-arm64/resources/native-binary/claude";
        let used = McpApp { path: elsewhere.into(), last_used: 20, ..code.clone() };
        write_mcp_app(&connection, &used).expect("write again");
        assert_eq!(read_mcp_app(&connection, &code.key).expect("read one"), Some(used));
        assert_eq!(read_mcp_app(&connection, "/nowhere").expect("read none"), None);
        connection.execute("DELETE FROM mcp_apps WHERE key = ?1", params![code.key]).expect("remove");
        assert_eq!(read_mcp_apps(&connection).expect("read").len(), 1);
        connection.execute("DELETE FROM mcp_apps", []).expect("clear");
        assert_eq!(read_mcp_apps(&connection).expect("read"), vec![]);
        drop(connection);
        let _ = std::fs::remove_dir_all(path.parent().expect("the folder"));
    }

    #[test]
    fn ssh_servers_keys_are_kept_by_host_and_port() {
        let (path, backup) = scratch("ssh");
        let connection = open_at(&path, &a_key(10), &backup).expect("a new database");
        assert_eq!(read_ssh_host_key(&connection, "sb-ams.example.net", 22).expect("read"), None);
        let save = |host: &str, port: u16, key: &str| {
            connection
                .execute("INSERT INTO ssh_hosts (host, port, key) VALUES (?1, ?2, ?3) ON CONFLICT(host, port) DO UPDATE SET key = excluded.key", params![host, i64::from(port), key])
                .expect("saved")
        };
        save("sb-ams.example.net", 22, "ssh-ed25519 AAAA1");
        save("sb-ams.example.net", 2222, "ssh-ed25519 AAAA2");
        assert_eq!(read_ssh_host_key(&connection, "sb-ams.example.net", 22).expect("read"), Some("ssh-ed25519 AAAA1".into()));
        assert_eq!(read_ssh_host_key(&connection, "sb-ams.example.net", 2222).expect("read"), Some("ssh-ed25519 AAAA2".into()));
        save("sb-ams.example.net", 22, "ssh-ed25519 BBBB");
        assert_eq!(read_ssh_host_key(&connection, "sb-ams.example.net", 22).expect("read"), Some("ssh-ed25519 BBBB".into()));
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
