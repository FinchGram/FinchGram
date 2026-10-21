//! The app's own database: SQLite, encrypted as a whole with SQLCipher, for what FinchGram keeps
//! that is neither a preference (settings.toml is for those, and only those) nor Telegram's (that
//! is TDLib's database): the WebDAV servers with their passwords, the programs the user has
//! allowed or turned away at AI access, and whatever comes next. It is
//! `finchgram.db` next to settings.toml. Its key is random, made the first time and kept in the
//! Keychain under FinchGram's name (platform::KEYCHAIN_SERVICE, the account "database"), where
//! only this app reads it; TDLib's database key is kept the same way (src/telegram/mod.rs).
//! SQLCipher is built from the rusqlite crate's bundled sources, with Apple's CommonCrypto on
//! macOS. One connection, on the UI thread.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use rusqlite::{Connection, OptionalExtension as _, params};

use crate::platform;

/// The Keychain account of the database's key.
const KEY_ACCOUNT: &str = "database";

thread_local! {
    static DATABASE: RefCell<Option<Connection>> = const { RefCell::new(None) };
}

/// A WebDAV server of the downloads page, as the user named it (docs/downloads.md).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebdavServer {
    pub name: String,
    /// "https://nas.home:5006/dav", without a trailing slash; a folder may be part of it.
    pub url: String,
    /// The account; "" for a server that asks for none.
    pub username: String,
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

/// Open the database, making it and its key the first time. Call once, on the UI thread, before
/// anything reads it. Err when it cannot be opened: the Keychain gave no key, or the file is
/// encrypted with another key than the Keychain's; then nothing of it is read or written, and
/// the error says why.
pub fn open() -> Result<(), String> {
    let path = file().ok_or("the system has no folder for FinchGram's database")?;
    let connection = open_at(&path, &key()?)?;
    DATABASE.with(|slot| *slot.borrow_mut() = Some(connection));
    Ok(())
}

/// The key from the Keychain, or a new one kept there.
fn key() -> Result<String, String> {
    match platform::keychain_read(platform::KEYCHAIN_SERVICE, KEY_ACCOUNT) {
        Ok(Some(key)) if !key.is_empty() => return Ok(key),
        Ok(_) => {}
        Err(err) => return Err(format!("cannot read the database key from the Keychain: {err}")),
    }
    let key = platform::random_key().ok_or("the system gave no random bytes for a key")?;
    platform::keychain_store(platform::KEYCHAIN_SERVICE, KEY_ACCOUNT, &key).map_err(|err| format!("cannot keep the database key in the Keychain: {err}"))?;
    Ok(key)
}

/// `path` opened with `key` (32 bytes as base64), its tables made when they are not there.
fn open_at(path: &Path, key: &str) -> Result<Connection, String> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|err| format!("cannot create {}: {err}", folder.display()))?;
    }
    let raw = base64::engine::general_purpose::STANDARD.decode(key).map_err(|err| format!("the database key is not base64: {err}"))?;
    if raw.len() != 32 {
        return Err(format!("the database key is {} bytes, not 32", raw.len()));
    }
    let hex: String = raw.iter().map(|byte| format!("{byte:02x}")).collect();
    let connection = Connection::open(path).map_err(|err| format!("cannot open {}: {err}", path.display()))?;
    // SQLCipher takes the key raw, as hex: the bytes themselves, with no passphrase to derive from.
    connection.execute_batch(&format!("PRAGMA key = \"x'{hex}'\";")).map_err(|err| format!("cannot key the database: {err}"))?;
    // A wrong key, or a file that is not a database, shows at the first read.
    connection
        .query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))
        .map_err(|err| format!("{} cannot be read: is its key the one in the Keychain? ({err})", path.display()))?;
    connection
        .execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS webdav_servers (
                 id INTEGER PRIMARY KEY,
                 position INTEGER NOT NULL,
                 name TEXT NOT NULL,
                 url TEXT NOT NULL,
                 username TEXT NOT NULL,
                 password TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS mcp_apps (
                 key TEXT PRIMARY KEY,
                 path TEXT NOT NULL,
                 name TEXT NOT NULL,
                 allowed INTEGER NOT NULL,
                 last_used INTEGER NOT NULL
             );",
        )
        .map_err(|err| format!("cannot make the tables: {err}"))?;
    Ok(connection)
}

/// Where the database is: next to settings.toml (src/settings.rs).
fn file() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("FinchGram").join("finchgram.db"))
}

/// Something with the open database; none when it is not open.
fn with<R>(work: impl FnOnce(&Connection) -> rusqlite::Result<R>) -> Result<R, String> {
    DATABASE.with(|slot| {
        let slot = slot.borrow();
        let connection = slot.as_ref().ok_or("the database is not open")?;
        work(connection).map_err(|err| err.to_string())
    })
}

/// The WebDAV servers, in the order the user keeps them.
pub fn webdav_servers() -> Vec<WebdavServer> {
    let read = |connection: &Connection| read_webdav_servers(connection);
    match with(read) {
        Ok(servers) => servers,
        Err(err) => {
            eprintln!("database: cannot read the WebDAV servers: {err}");
            Vec::new()
        }
    }
}

/// Keep `servers` as the WebDAV servers, in this order, in place of the ones kept before.
pub fn save_webdav_servers(servers: &[WebdavServer]) -> Result<(), String> {
    with(|connection| write_webdav_servers(connection, servers))
}

fn read_webdav_servers(connection: &Connection) -> rusqlite::Result<Vec<WebdavServer>> {
    let mut statement = connection.prepare("SELECT name, url, username, password FROM webdav_servers ORDER BY position, id")?;
    let rows = statement.query_map([], |row| Ok(WebdavServer { name: row.get(0)?, url: row.get(1)?, username: row.get(2)?, password: row.get(3)? }))?;
    rows.collect()
}

fn write_webdav_servers(connection: &Connection, servers: &[WebdavServer]) -> rusqlite::Result<()> {
    connection.execute_batch("BEGIN; DELETE FROM webdav_servers;")?;
    let written: rusqlite::Result<()> = servers.iter().enumerate().try_for_each(|(position, server)| {
        connection
            .execute(
                "INSERT INTO webdav_servers (position, name, url, username, password) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![position as i64, server.name, server.url, server.username, server.password],
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

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("finchgram-database-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        folder.join("finchgram.db")
    }

    fn a_key(byte: u8) -> String {
        base64::engine::general_purpose::STANDARD.encode([byte; 32])
    }

    #[test]
    fn servers_are_kept_in_order_and_read_back_with_the_key_only() {
        let path = scratch("servers");
        let servers = vec![
            WebdavServer { name: "Home NAS".into(), url: "https://nas.home:5006/dav".into(), username: "me".into(), password: "secret".into() },
            WebdavServer { name: "PikPak".into(), url: "https://dav.pikpak.ai".into(), username: "x".into(), password: "y".into() },
        ];
        {
            let connection = open_at(&path, &a_key(1)).expect("a new database");
            assert_eq!(read_webdav_servers(&connection).expect("read"), vec![]);
            write_webdav_servers(&connection, &servers).expect("write");
            let fewer = &servers[1..];
            write_webdav_servers(&connection, fewer).expect("write again");
            assert_eq!(read_webdav_servers(&connection).expect("read"), fewer);
            write_webdav_servers(&connection, &servers).expect("write all");
        }
        // The same key opens it; another does not, and nor does none.
        let connection = open_at(&path, &a_key(1)).expect("opened again");
        assert_eq!(read_webdav_servers(&connection).expect("read"), servers);
        drop(connection);
        assert!(open_at(&path, &a_key(2)).is_err());
        let plain = Connection::open(&path).expect("the file");
        assert!(plain.query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get::<_, i64>(0)).is_err());
        // The file holds no plain text of what is in it.
        let bytes = std::fs::read(&path).expect("the file's bytes");
        assert!(!bytes.windows(6).any(|window| window == b"secret"));
        assert!(!bytes.windows(14).any(|window| window == b"webdav_servers"));
        let _ = std::fs::remove_dir_all(path.parent().expect("the folder"));
    }

    #[test]
    fn programs_are_kept_by_their_key_and_forgotten_one_by_one_or_all_at_once() {
        let path = scratch("apps");
        let connection = open_at(&path, &a_key(3)).expect("a new database");
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
    fn a_key_must_be_thirty_two_bytes_of_base64() {
        let path = scratch("key");
        assert!(open_at(&path, "not base64!").is_err());
        assert!(open_at(&path, &base64::engine::general_purpose::STANDARD.encode([7u8; 16])).is_err());
        let _ = std::fs::remove_dir_all(path.parent().expect("the folder"));
    }
}
