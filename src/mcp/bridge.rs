//! `finchgram mcp`: the end of the MCP server that an assistant's app runs. Such apps (Claude
//! Desktop, Claude Code, Cursor, …) start an MCP server as a program of their own and talk to it
//! over its standard input and output, one JSON-RPC message per line. The server itself is the
//! running FinchGram, listening on its socket (mod.rs): this program only carries the lines between
//! the two, and starts FinchGram when it is not running. It writes nothing of its own to standard
//! output, which the protocol keeps for messages; its own words go to standard error.

use std::io::{self, BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// How long a FinchGram that was just started may take to listen.
const START_TIMEOUT: Duration = Duration::from_secs(30);
const RETRY: Duration = Duration::from_millis(250);

/// Runs until the assistant's app closes its end, or FinchGram does. The process's exit code.
pub fn run() -> i32 {
    let path = super::socket_path();
    let stream = match connect(&path) {
        Ok(stream) => stream,
        Err(err) => {
            eprintln!("finchgram mcp: {err}");
            return 1;
        }
    };
    let from_app = match stream.try_clone() {
        Ok(reading) => reading,
        Err(err) => {
            eprintln!("finchgram mcp: cannot read from FinchGram: {err}");
            return 1;
        }
    };
    let to_stdout = thread::spawn(move || {
        let mut out = io::stdout().lock();
        for line in BufReader::new(from_app).lines() {
            let Ok(line) = line else { break };
            if writeln!(out, "{line}").and_then(|()| out.flush()).is_err() {
                break;
            }
        }
    });
    let mut to_app = stream;
    for line in io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        if writeln!(to_app, "{line}").is_err() {
            break;
        }
    }
    // The app closed its input: we are done, whatever FinchGram still had to say.
    let _ = to_app.shutdown(Shutdown::Both);
    let _ = to_stdout.join();
    0
}

/// FinchGram's socket, starting FinchGram when nothing listens.
fn connect(path: &Path) -> Result<UnixStream, String> {
    if let Ok(stream) = UnixStream::connect(path) {
        return Ok(stream);
    }
    start_app()?;
    let until = Instant::now() + START_TIMEOUT;
    loop {
        match UnixStream::connect(path) {
            Ok(stream) => return Ok(stream),
            Err(_) if Instant::now() < until => thread::sleep(RETRY),
            Err(err) => {
                return Err(format!(
                    "FinchGram did not start listening at {} within {} seconds: {err}",
                    path.display(),
                    START_TIMEOUT.as_secs()
                ));
            }
        }
    }
}

/// Start FinchGram: the app this program is part of, in the background; or, in development, the
/// bare executable.
fn start_app() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|err| format!("cannot tell where FinchGram is: {err}"))?;
    match super::peer::bundle_of(&exe) {
        Some(bundle) => {
            eprintln!("finchgram mcp: starting {}", bundle.display());
            let status = Command::new("/usr/bin/open")
                .arg("-g")
                .arg("-a")
                .arg(&bundle)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map_err(|err| format!("cannot start {}: {err}", bundle.display()))?;
            if !status.success() {
                return Err(format!("cannot start {}: open said {status}", bundle.display()));
            }
        }
        None => {
            eprintln!("finchgram mcp: starting {}", exe.display());
            Command::new(&exe)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|err| format!("cannot start {}: {err}", exe.display()))?;
        }
    }
    Ok(())
}
