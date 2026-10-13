//! Who is at the other end of a connection to the socket: the process that connected, and when
//! that is our own bridge (`finchgram mcp`), the program that started it, which is the assistant's
//! app (Claude Desktop, Claude Code, …). Nothing is taken from what connects: the system is asked
//! who it is (the socket's LOCAL_PEERPID, then libproc for the program and its parent), so nothing
//! connecting can claim to be something else. The name goes into the log, into Settings → AI
//! access's list of apps, and into the dialog that asks the user to allow an app (access.rs).

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

/// Shells and launchers an assistant's app may start the bridge through: the program behind them
/// is the one to name.
const WRAPPERS: &[&str] = &["sh", "bash", "zsh", "dash", "fish", "env"];
/// How far up the parents to look for the program behind the bridge.
const ANCESTORS: usize = 6;

/// The program at the other end.
#[derive(Debug, Clone, PartialEq)]
pub struct Host {
    pub pid: i32,
    pub path: Option<PathBuf>,
}

impl Host {
    /// "Claude (/Applications/Claude.app)", "claude (/Users/x/.local/bin/claude)", or only the
    /// process id when the system would not say more.
    pub fn describe(&self) -> String {
        match &self.path {
            Some(_) => format!("{} ({})", self.name(), self.key()),
            None if self.pid > 0 => format!("pid {}", self.pid),
            None => "an unknown process".to_string(),
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

    /// What the user's answer is remembered by: the app bundle ("/Applications/Claude.app"), the
    /// folder of a program kept under its version number ("/Users/x/.local/share/claude", so that
    /// an update of Claude Code is not a stranger), or the program's path; for a program the
    /// system would not name, its process id, which lasts as long as it runs.
    pub fn key(&self) -> String {
        match &self.path {
            Some(path) => bundle_of(path).or_else(|| versioned_home(path)).unwrap_or_else(|| path.clone()).display().to_string(),
            None if self.pid > 0 => format!("pid {}", self.pid),
            None => "unknown".to_string(),
        }
    }

    /// The system named the program: its answer can be kept for next time.
    pub fn identified(&self) -> bool {
        self.path.is_some()
    }
}

/// The program behind `stream`: the process that connected, unless that is our own bridge or a
/// shell, in which case the one that started it.
pub fn host_of(stream: &UnixStream) -> Host {
    let Some(peer) = system::peer_pid(stream) else { return Host { pid: 0, path: None } };
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
    Host { pid, path: if ours_or_wrapper { None } else { path } }
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

/// For a program kept under its version number, the folder that holds every version:
/// "/Users/x/.local/share/claude" for "/Users/x/.local/share/claude/versions/2.1.296".
fn versioned_home(path: &Path) -> Option<PathBuf> {
    if !is_version(path.file_name()?.to_str()?) {
        return None;
    }
    let parent = path.parent()?;
    let home = if parent.file_name().and_then(|name| name.to_str()) == Some("versions") { parent.parent()? } else { parent };
    Some(home.to_path_buf())
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

    /// The process that started `pid`.
    pub fn parent_of(pid: i32) -> Option<i32> {
        let mut info = MaybeUninit::<ProcBsdInfo>::zeroed();
        let size = size_of::<ProcBsdInfo>() as i32;
        // SAFETY: the buffer is exactly a proc_bsdinfo; the call fills it whole or returns less.
        let written = unsafe { proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, info.as_mut_ptr().cast(), size) };
        if written != size {
            return None;
        }
        // SAFETY: the system wrote the whole structure.
        let info = unsafe { info.assume_init() };
        i32::try_from(info.pbi_ppid).ok()
    }
}

#[cfg(not(target_os = "macos"))]
mod system {
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;

    pub fn peer_pid(_stream: &UnixStream) -> Option<i32> {
        None
    }

    pub fn path_of(_pid: i32) -> Option<PathBuf> {
        None
    }

    pub fn parent_of(_pid: i32) -> Option<i32> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_program_in_an_app_bundle_is_named_after_the_bundle() {
        let claude = Host { pid: 7, path: Some(PathBuf::from("/Applications/Claude.app/Contents/MacOS/Claude")) };
        assert_eq!(claude.describe(), "Claude (/Applications/Claude.app)");
        let tool = Host { pid: 8, path: Some(PathBuf::from("/Users/x/.local/bin/claude")) };
        assert_eq!(tool.describe(), "claude (/Users/x/.local/bin/claude)");
        assert_eq!((claude.name(), claude.key()), ("Claude".to_string(), "/Applications/Claude.app".to_string()));
        assert_eq!((tool.name(), tool.key()), ("claude".to_string(), "/Users/x/.local/bin/claude".to_string()));
        let nameless = Host { pid: 9, path: None };
        assert_eq!((nameless.describe(), nameless.name(), nameless.key(), nameless.identified()), ("pid 9".into(), String::new(), "pid 9".into(), false));
        assert_eq!(Host { pid: 0, path: None }.describe(), "an unknown process");
        assert_eq!(Host { pid: 0, path: None }.key(), "unknown");
        assert_eq!(bundle_of(Path::new("/usr/bin/node")), None);
        let versioned = Host { pid: 10, path: Some(PathBuf::from("/Users/x/.local/share/claude/versions/2.1.296")) };
        assert_eq!((versioned.name(), versioned.key()), ("claude".to_string(), "/Users/x/.local/share/claude".to_string()));
        assert_eq!(versioned.describe(), "claude (/Users/x/.local/share/claude)");
        assert_eq!(program_name(Path::new("/opt/homebrew/bin/node")), "node");
        assert_eq!(program_name(Path::new("/Users/x/tools/3.0")), "tools");
        assert_eq!(versioned_home(Path::new("/Users/x/tools/3.0")), Some(PathBuf::from("/Users/x/tools")));
        assert_eq!(versioned_home(Path::new("/opt/homebrew/bin/node")), None);
        assert_eq!(program_name(Path::new("/")), "program");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_system_knows_this_process_and_its_parent() {
        let me = std::process::id() as i32;
        let path = system::path_of(me).expect("our own path");
        assert_eq!(path.canonicalize().ok(), std::env::current_exe().ok().and_then(|exe| exe.canonicalize().ok()));
        let parent = system::parent_of(me).expect("a parent");
        assert!(parent > 0 && parent != me);
        let (ours, theirs) = UnixStream::pair().expect("a pair");
        assert_eq!(system::peer_pid(&ours), Some(me));
        drop(theirs);
    }
}
