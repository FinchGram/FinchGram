//! Who is at the other end of a connection to the socket: the process that connected, and when
//! that is our own bridge (`finchgram mcp`), the program that started it, which is the assistant's
//! app (Claude Desktop, Claude Code, …). Nothing is taken from what connects: the system is asked
//! who it is (the socket's LOCAL_PEERPID, then libproc for the program and its parent, then the
//! Security framework for the program's code signature), so nothing connecting can claim to be
//! something else. The name and the place go into the log, into Settings → AI access's list of
//! apps, and into the dialog that asks the user to allow an app (access.rs); the signature is what
//! the user's answer is remembered by, or, for a program without one from a team, its path and the
//! hash of its code.

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

/// Shells and launchers an assistant's app may start the bridge through: the program behind them
/// is the one to name.
const WRAPPERS: &[&str] = &["sh", "bash", "zsh", "dash", "fish", "env"];
/// How far up the parents to look for the program behind the bridge.
const ANCESTORS: usize = 6;
/// Programs that run whatever script they are given. The system names the interpreter, not the
/// script (which is only an argument, and one the script can lie about through the interpreter's
/// own switches and variables), so an answer kept for one would stand for every script anyone
/// runs with it: theirs are kept for the run and the process only (`Host::kept`), as for a
/// program the system would not name. By the file's name, with its version taken off:
/// "python3.12", "Python" (the one inside Python.framework), "ruby3.3".
const INTERPRETERS: &[&str] = &[
    "node", "nodejs", "bun", "deno", "electron", "python", "pythonw", "ruby", "perl", "php", "java", "osascript", "lua", "luajit", "tclsh", "wish", "rscript", "julia", "pwsh",
    "powershell", "dotnet",
];

/// A program's code signature, as the system validated it, under a certificate Apple issued to a
/// team (Developer ID or the App Store). It is the same whatever the program's version and
/// wherever it is installed, which a path is not: Claude Code is kept under its version number,
/// and by the VS Code extension under another folder again. macOS itself tells apps apart this
/// way (the designated requirements its privacy settings are kept by).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    /// The developer's Team ID: "Q6L2SF6YDW" for Anthropic PBC.
    pub team: String,
    /// The signing identifier the developer gave the program: "com.anthropic.claude-code",
    /// "com.anthropic.claudefordesktop".
    pub identifier: String,
}

impl Signature {
    /// "Q6L2SF6YDW/com.anthropic.claude-code": the team, then the identifier it chose.
    pub fn key(&self) -> String {
        format!("{}/{}", self.team, self.identifier)
    }
}

/// The program at the other end.
#[derive(Debug, Clone, PartialEq)]
pub struct Host {
    pub pid: i32,
    pub path: Option<PathBuf>,
    /// Its code signature, when it has a valid one from a team.
    pub signature: Option<Signature>,
    /// The cdhash of its code (src/platform/), for a program without a signature from a team:
    /// what its answer is held to besides its path (`identity`). None for one with such a
    /// signature, which needs none, and for one with no signature at all.
    pub hash: Option<String>,
    /// When the process started, in seconds since the epoch; 0 when the system would not say. A
    /// program the system would not name is remembered for the run by its pid and this: a later
    /// process under the same pid (pids are reused) is a stranger (access.rs).
    pub started: u64,
}

impl Host {
    /// "Claude (/Applications/Claude.app, Q6L2SF6YDW/com.anthropic.claudefordesktop)",
    /// "node (/opt/homebrew/bin/node)", or only the process id when the system would not say more.
    pub fn describe(&self) -> String {
        match (&self.path, &self.signature) {
            (Some(_), Some(signature)) => format!("{} ({}, {})", self.name(), self.location(), signature.key()),
            (Some(_), None) => format!("{} ({})", self.name(), self.location()),
            (None, _) if self.pid > 0 => format!("pid {}", self.pid),
            (None, _) => "an unknown process".to_string(),
        }
    }

    /// The program's name as the user sees it: its app bundle's ("Claude"), or its file name
    /// ("claude"); empty when the system would not say which program it is. A program kept under
    /// its version number ("…/claude/versions/2.1.296", as Claude Code is) goes by the folder that
    /// names it.
    pub fn name(&self) -> String {
        match &self.path {
            Some(path) => match bundle_of(path) {
                Some(bundle) => bundle.file_stem().and_then(|stem| stem.to_str()).unwrap_or("app").to_string(),
                None => program_name(path),
            },
            None => String::new(),
        }
    }

    /// Where the program is, as the user sees it: its app bundle ("/Applications/Claude.app") or
    /// the program itself ("/Users/x/.local/share/claude/versions/2.1.296"); for a program the
    /// system would not name, its process id.
    pub fn location(&self) -> String {
        match &self.path {
            Some(path) => bundle_of(path).unwrap_or_else(|| path.clone()).display().to_string(),
            None if self.pid > 0 => format!("pid {}", self.pid),
            None => "unknown".to_string(),
        }
    }

    /// What the user's answer is remembered by: the program's signature
    /// ("Q6L2SF6YDW/com.anthropic.claude-code"), the same for every version and every copy of it;
    /// for a program without one from a team, the program itself, by its path, and not its app
    /// bundle, which may hold other programs that are not it (`host_of` has the path with its
    /// links followed); for one the system would not name, its process id, which lasts as long
    /// as it runs.
    pub fn key(&self) -> String {
        match (&self.signature, &self.path) {
            (Some(signature), _) => signature.key(),
            (None, Some(path)) => path.display().to_string(),
            (None, None) => self.location(),
        }
    }

    /// The system named the program.
    pub fn identified(&self) -> bool {
        self.path.is_some()
    }

    /// The user's answer about the program can be kept for next time: the system named it, it is
    /// not an interpreter (INTERPRETERS), whose name says nothing about the script it runs, and
    /// there is something to hold the answer to, a signature from a team or the hash of its code.
    /// Otherwise the answer lasts for the run and this process only (access.rs).
    pub fn kept(&self) -> bool {
        self.path.as_ref().is_some_and(|path| !is_interpreter(path)) && (self.signature.is_some() || self.hash.is_some())
    }

    /// What a kept answer is held to besides the key: nothing for a program with a signature from
    /// a team, which is its key, the same through every update; for one without, the cdhash of
    /// its code, which an update changes, so that the program at the path is asked about again
    /// (access.rs), and one put in its place is not taken for it.
    pub fn identity(&self) -> String {
        if self.signature.is_some() { String::new() } else { self.hash.clone().unwrap_or_default() }
    }
}

/// Whether `path` names one of INTERPRETERS: its file name in small letters, any version number
/// at the end taken off.
fn is_interpreter(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else { return false };
    let name = name.to_ascii_lowercase();
    let base = name.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    INTERPRETERS.contains(&base)
}

/// The program behind `stream`: the process that connected, unless that is our own bridge or a
/// shell, in which case the one that started it.
pub fn host_of(stream: &UnixStream) -> Host {
    let Some(peer) = system::peer_pid(stream) else { return Host { pid: 0, path: None, signature: None, hash: None, started: 0 } };
    let ours = std::env::current_exe().ok().and_then(|exe| exe.canonicalize().ok());
    let mut pid = peer;
    let mut path = system::path_of(pid);
    for _ in 0..ANCESTORS {
        let Some(current) = &path else { break };
        let is_ours = ours.as_ref().is_some_and(|ours| current.canonicalize().ok().as_ref() == Some(ours));
        let is_wrapper = current.file_name().and_then(|name| name.to_str()).is_some_and(|name| WRAPPERS.contains(&name));
        if !is_ours && !is_wrapper {
            break;
        }
        let Some(parent) = system::parent_of(pid).filter(|parent| *parent > 1) else { break };
        pid = parent;
        path = system::path_of(pid);
    }
    // Only our own bridge, or a shell, could be found (its parent is gone, or further away): that
    // is not the program.
    let ours_or_wrapper = path.as_ref().is_some_and(|current| {
        ours.as_ref().is_some_and(|ours| current.canonicalize().ok().as_ref() == Some(ours))
            || current.file_name().and_then(|name| name.to_str()).is_some_and(|name| WRAPPERS.contains(&name))
    });
    // The path with its links followed: a program without a signature is remembered by it.
    let path = if ours_or_wrapper { None } else { path.map(|path| path.canonicalize().unwrap_or(path)) };
    let signature = if path.is_some() { system::signature_of(pid) } else { None };
    // A program without a signature from a team is held to its code besides its path.
    let hash = if path.is_some() && signature.is_none() { system::code_hash(pid) } else { None };
    Host { pid, path, signature, hash, started: system::started_at(pid).unwrap_or(0) }
}

/// Whether the process at the other end of `stream` is this very program: for the bridge
/// (`finchgram mcp`), which connects to FinchGram's socket, so that another program of the user's
/// that took the socket's place cannot stand in for FinchGram (it would see what the assistant
/// asks for, and answer it). Told the way the server tells apps apart: the same signature from a
/// team (the release); without one (development), the same executable.
pub fn verify_server(stream: &UnixStream) -> Result<(), String> {
    let pid = system::peer_pid(stream).ok_or("the system would not say which process listens at FinchGram's socket")?;
    let theirs = system::path_of(pid).map(|path| path.canonicalize().unwrap_or(path));
    let where_ = theirs.as_ref().map_or_else(|| format!("pid {pid}"), |path| path.display().to_string());
    match (system::signature_of(std::process::id() as i32), system::signature_of(pid)) {
        (Some(mine), Some(its)) if mine == its => Ok(()),
        (Some(_), _) => Err(format!("the process listening at FinchGram's socket is not FinchGram ({where_})")),
        (None, _) => {
            let mine = std::env::current_exe().ok().and_then(|exe| exe.canonicalize().ok());
            if mine.is_some() && mine == theirs { Ok(()) } else { Err(format!("the process listening at FinchGram's socket is not this FinchGram ({where_})")) }
        }
    }
}

/// The app bundle a program is in, when it is in one: "/Applications/Claude.app" for
/// "/Applications/Claude.app/Contents/MacOS/Claude".
pub fn bundle_of(path: &Path) -> Option<PathBuf> {
    path.ancestors().find(|ancestor| ancestor.extension().is_some_and(|extension| extension == "app")).map(Path::to_path_buf)
}

/// "2.1.296": a version number, as a file or folder is named by one.
fn is_version(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_digit() || c == '.')
}

/// A program's name from its path: its file name, unless that is a version number, in which case
/// the first folder above it that is not "versions" or "bin" ("claude" for
/// "/Users/x/.local/share/claude/versions/2.1.296").
fn program_name(path: &Path) -> String {
    let names = path.iter().rev().filter_map(|part| part.to_str());
    let mut names = names.skip_while(|name| is_version(name) || matches!(*name, "versions" | "bin"));
    match (path.file_name().and_then(|name| name.to_str()), names.next()) {
        (Some(file), _) if !is_version(file) => file.to_string(),
        (_, Some(folder)) if folder != "/" => folder.to_string(),
        (Some(file), _) => file.to_string(),
        (None, _) => "program".to_string(),
    }
}

#[cfg(target_os = "macos")]
mod system {
    use std::ffi::c_void;
    use std::mem::MaybeUninit;
    use std::os::fd::AsRawFd;
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;

    use super::Signature;

    unsafe extern "C" {
        fn getsockopt(socket: i32, level: i32, option_name: i32, option_value: *mut c_void, option_len: *mut u32) -> i32;
        fn proc_pidpath(pid: i32, buffer: *mut c_void, buffersize: u32) -> i32;
        fn proc_pidinfo(pid: i32, flavor: i32, arg: u64, buffer: *mut c_void, buffersize: i32) -> i32;
    }

    const SOL_LOCAL: i32 = 0;
    const LOCAL_PEERPID: i32 = 2;
    const PROC_PIDTBSDINFO: i32 = 3;
    /// PROC_PIDPATHINFO_MAXSIZE: four times MAXPATHLEN.
    const PATH_BUFFER: usize = 4 * 1024;

    /// libproc's proc_bsdinfo, as far as its parent process id.
    #[repr(C)]
    #[allow(dead_code)]
    struct ProcBsdInfo {
        pbi_flags: u32,
        pbi_status: u32,
        pbi_xstatus: u32,
        pbi_pid: u32,
        pbi_ppid: u32,
        pbi_uid: u32,
        pbi_gid: u32,
        pbi_ruid: u32,
        pbi_rgid: u32,
        pbi_svuid: u32,
        pbi_svgid: u32,
        rfu_1: u32,
        pbi_comm: [u8; 16],
        pbi_name: [u8; 32],
        pbi_nfiles: u32,
        pbi_pgid: u32,
        pbi_pjobc: u32,
        e_tdev: u32,
        e_tpgid: u32,
        pbi_nice: i32,
        pbi_start_tvsec: u64,
        pbi_start_tvusec: u64,
    }

    /// The process at the other end of the socket.
    pub fn peer_pid(stream: &UnixStream) -> Option<i32> {
        let mut pid: i32 = 0;
        let mut length = size_of::<i32>() as u32;
        // SAFETY: a valid socket descriptor, and the buffer is as long as `length` says.
        let result = unsafe { getsockopt(stream.as_raw_fd(), SOL_LOCAL, LOCAL_PEERPID, std::ptr::from_mut(&mut pid).cast(), &mut length) };
        (result == 0 && pid > 0).then_some(pid)
    }

    /// The program a process runs.
    pub fn path_of(pid: i32) -> Option<PathBuf> {
        let mut buffer = vec![0u8; PATH_BUFFER];
        // SAFETY: the buffer is as long as said; proc_pidpath writes at most that many bytes.
        let length = unsafe { proc_pidpath(pid, buffer.as_mut_ptr().cast(), PATH_BUFFER as u32) };
        if length <= 0 {
            return None;
        }
        buffer.truncate(length as usize);
        Some(PathBuf::from(String::from_utf8_lossy(&buffer).into_owned()))
    }

    /// What the system knows of the process `pid`.
    fn bsd_info(pid: i32) -> Option<ProcBsdInfo> {
        let mut info = MaybeUninit::<ProcBsdInfo>::zeroed();
        let size = size_of::<ProcBsdInfo>() as i32;
        // SAFETY: the buffer is exactly a proc_bsdinfo; the call fills it whole or returns less.
        let written = unsafe { proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, info.as_mut_ptr().cast(), size) };
        if written != size {
            return None;
        }
        // SAFETY: the system wrote the whole structure.
        Some(unsafe { info.assume_init() })
    }

    /// The process that started `pid`.
    pub fn parent_of(pid: i32) -> Option<i32> {
        i32::try_from(bsd_info(pid)?.pbi_ppid).ok()
    }

    /// When the process `pid` started, in seconds since the epoch.
    pub fn started_at(pid: i32) -> Option<u64> {
        Some(bsd_info(pid)?.pbi_start_tvsec)
    }

    /// The code signature of the process `pid`, as the system validates it under a certificate
    /// Apple issued (src/platform/): its identifier and, when the certificate names one, its team.
    /// None for a program without such a signature, or one that is gone.
    pub fn signing_of(pid: i32) -> Option<(String, Option<String>)> {
        crate::platform::code_signature(pid)
    }

    /// The signature a program's answer is remembered by: one from a team.
    pub fn signature_of(pid: i32) -> Option<Signature> {
        let (identifier, team) = signing_of(pid)?;
        Some(Signature { team: team?, identifier })
    }

    /// The cdhash of the code the process `pid` runs (src/platform/), signed by a team or only ad
    /// hoc: None for a program with no signature at all, or one that is gone.
    pub fn code_hash(pid: i32) -> Option<String> {
        crate::platform::code_hash(pid)
    }
}

#[cfg(not(target_os = "macos"))]
mod system {
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;

    use super::Signature;

    pub fn peer_pid(_stream: &UnixStream) -> Option<i32> {
        None
    }

    pub fn path_of(_pid: i32) -> Option<PathBuf> {
        None
    }

    pub fn parent_of(_pid: i32) -> Option<i32> {
        None
    }

    pub fn started_at(_pid: i32) -> Option<u64> {
        None
    }

    pub fn signature_of(_pid: i32) -> Option<Signature> {
        None
    }

    pub fn code_hash(_pid: i32) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cdhash, as the system gives it.
    fn hash() -> String {
        "4f".repeat(20)
    }

    #[test]
    fn a_program_in_an_app_bundle_is_named_after_the_bundle() {
        let claude = Host { pid: 7, path: Some(PathBuf::from("/Applications/Claude.app/Contents/MacOS/Claude")), signature: None, hash: None, started: 0 };
        assert_eq!(claude.describe(), "Claude (/Applications/Claude.app)");
        let tool = Host { pid: 8, path: Some(PathBuf::from("/Users/x/.local/bin/claude")), signature: None, hash: None, started: 0 };
        assert_eq!(tool.describe(), "claude (/Users/x/.local/bin/claude)");
        // Without a signature from a team, the program itself is the key, not its bundle: two
        // programs in one unsigned bundle are two answers.
        assert_eq!((claude.name(), claude.location(), claude.key()), ("Claude".to_string(), "/Applications/Claude.app".to_string(), "/Applications/Claude.app/Contents/MacOS/Claude".to_string()));
        let helper = Host { pid: 7, path: Some(PathBuf::from("/Applications/Claude.app/Contents/Helpers/other")), signature: None, hash: None, started: 0 };
        assert_eq!((helper.name(), helper.location()), (claude.name(), claude.location()));
        assert_ne!(helper.key(), claude.key());
        assert_eq!((tool.name(), tool.key()), ("claude".to_string(), "/Users/x/.local/bin/claude".to_string()));
        let nameless = Host { pid: 9, path: None, signature: None, hash: None, started: 0 };
        assert_eq!((nameless.describe(), nameless.name(), nameless.key(), nameless.identified()), ("pid 9".into(), String::new(), "pid 9".into(), false));
        assert_eq!(Host { pid: 0, path: None, signature: None, hash: None, started: 0 }.describe(), "an unknown process");
        assert_eq!(Host { pid: 0, path: None, signature: None, hash: None, started: 0 }.key(), "unknown");
        assert_eq!(bundle_of(Path::new("/usr/bin/node")), None);
        let versioned = Host { pid: 10, path: Some(PathBuf::from("/Users/x/.local/share/claude/versions/2.1.296")), signature: None, hash: None, started: 0 };
        assert_eq!((versioned.name(), versioned.key()), ("claude".to_string(), "/Users/x/.local/share/claude/versions/2.1.296".to_string()));
        assert_eq!(program_name(Path::new("/opt/homebrew/bin/node")), "node");
        assert_eq!(program_name(Path::new("/Users/x/tools/3.0")), "tools");
        assert_eq!(program_name(Path::new("/")), "program");
    }

    #[test]
    fn an_interpreters_answer_is_not_kept_whatever_its_signature() {
        for path in [
            "/opt/homebrew/bin/node",
            "/usr/local/bin/node",
            "/opt/homebrew/Cellar/python@3.12/3.12.4/Frameworks/Python.framework/Versions/3.12/Resources/Python.app/Contents/MacOS/Python",
            "/usr/bin/python3",
            "/opt/homebrew/bin/python3.12",
            "/opt/homebrew/opt/ruby/bin/ruby",
            "/Users/x/.bun/bin/bun",
            "/usr/bin/osascript",
            "/usr/bin/java",
            "/opt/homebrew/bin/php8.3",
            "/Users/x/project/node_modules/electron/dist/Electron.app/Contents/MacOS/Electron",
        ] {
            assert!(is_interpreter(Path::new(path)), "{path}");
            let host = Host { pid: 20, path: Some(PathBuf::from(path)), signature: None, hash: Some(hash()), started: 5 };
            assert!(host.identified() && !host.kept(), "{path}");
        }
        // Signed by a team or not, the interpreter is still the interpreter.
        let signed = Signature { team: "HHGW6JJ4A4".into(), identifier: "node".into() };
        let node = Host { pid: 21, path: Some(PathBuf::from("/usr/local/bin/node")), signature: Some(signed), hash: None, started: 5 };
        assert!(!node.kept());
        for path in ["/Applications/Claude.app/Contents/MacOS/Claude", "/Users/x/.local/share/claude/versions/2.1.306", "/Users/x/bin/nodeish", "/usr/bin/javac", "/opt/homebrew/bin/node-gyp"] {
            assert!(!is_interpreter(Path::new(path)), "{path}");
            // Kept by the hash of its code; with no signature at all, there is nothing to keep
            // the answer by.
            assert!(Host { pid: 22, path: Some(PathBuf::from(path)), signature: None, hash: Some(hash()), started: 5 }.kept(), "{path}");
            assert!(!Host { pid: 22, path: Some(PathBuf::from(path)), signature: None, hash: None, started: 5 }.kept(), "{path}");
        }
        assert!(!Host { pid: 23, path: None, signature: None, hash: Some(hash()), started: 5 }.kept());
    }

    #[test]
    fn a_signed_program_is_remembered_by_its_signature_wherever_it_is() {
        let signature = Signature { team: "Q6L2SF6YDW".into(), identifier: "com.anthropic.claude-code".into() };
        let standalone = Host { pid: 11, path: Some(PathBuf::from("/Users/x/.local/share/claude/versions/2.1.306")), signature: Some(signature.clone()), hash: None, started: 0 };
        let inside_vscode = "/Users/x/.vscode/extensions/anthropic.claude-code-2.1.305-darwin-arm64/resources/native-binary/claude";
        let extension = Host { pid: 12, path: Some(PathBuf::from(inside_vscode)), signature: Some(signature), hash: None, started: 0 };
        assert_eq!(standalone.key(), "Q6L2SF6YDW/com.anthropic.claude-code");
        assert_eq!(extension.key(), standalone.key());
        assert_eq!((standalone.name(), standalone.location()), ("claude".to_string(), "/Users/x/.local/share/claude/versions/2.1.306".to_string()));
        assert_eq!((extension.name(), extension.location()), ("claude".to_string(), inside_vscode.to_string()));
        assert_eq!(standalone.describe(), "claude (/Users/x/.local/share/claude/versions/2.1.306, Q6L2SF6YDW/com.anthropic.claude-code)");
        let desktop = Host {
            pid: 13,
            path: Some(PathBuf::from("/Applications/Claude.app/Contents/MacOS/Claude")),
            signature: Some(Signature { team: "Q6L2SF6YDW".into(), identifier: "com.anthropic.claudefordesktop".into() }),
            hash: None,
            started: 0,
        };
        assert_eq!((desktop.name(), desktop.location(), desktop.key()), ("Claude".to_string(), "/Applications/Claude.app".to_string(), "Q6L2SF6YDW/com.anthropic.claudefordesktop".to_string()));
        // A program without a team's signature is held to its path and to its code: the hash is
        // its identity. A signed program's identity is nothing, its signature being its key.
        let tool = Host { pid: 14, path: Some(PathBuf::from("/Users/x/bin/tool")), signature: None, hash: Some(hash()), started: 0 };
        assert_eq!((tool.key(), tool.identity(), tool.kept()), ("/Users/x/bin/tool".to_string(), hash(), true));
        assert_eq!(tool.describe(), "tool (/Users/x/bin/tool)");
        assert_eq!((desktop.identity(), standalone.identity()), (String::new(), String::new()));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_system_knows_this_process_and_its_parent() {
        let me = std::process::id() as i32;
        let path = system::path_of(me).expect("our own path");
        assert_eq!(path.canonicalize().ok(), std::env::current_exe().ok().and_then(|exe| exe.canonicalize().ok()));
        let parent = system::parent_of(me).expect("a parent");
        assert!(parent > 0 && parent != me);
        assert!(system::started_at(me).is_some_and(|started| started > 1_600_000_000));
        let (ours, theirs) = UnixStream::pair().expect("a pair");
        assert_eq!(system::peer_pid(&ours), Some(me));
        // The other end is this program: what the bridge requires of FinchGram's socket.
        assert_eq!(verify_server(&ours), Ok(()));
        drop(theirs);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_system_validates_a_signature_apple_stands_behind_and_no_other() {
        // Apple's own sleep: signed by Apple, under no team.
        let mut sleep = std::process::Command::new("/bin/sleep").arg("30").spawn().expect("sleep");
        let pid = i32::try_from(sleep.id()).expect("a pid");
        assert_eq!(system::signing_of(pid), Some(("com.apple.sleep".to_string(), None)));
        assert_eq!(system::signature_of(pid), None);
        // Every program that runs has a cdhash: Apple's own, and this test program, signed ad hoc
        // by the linker under no certificate at all.
        let is_hash = |hash: Option<String>| hash.is_some_and(|hash| hash.len() == 40 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(is_hash(system::code_hash(pid)));
        let _ = sleep.kill();
        let _ = sleep.wait();
        assert_eq!(system::signing_of(std::process::id() as i32), None);
        assert!(is_hash(system::code_hash(std::process::id() as i32)));
        assert_eq!(system::signature_of(-1), None);
        assert_eq!(system::code_hash(-1), None);
    }
}
