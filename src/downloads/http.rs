//! The downloads' HTTP client: hyper on a tokio runtime of its own, speaking HTTP/2 where a server
//! offers it (ALPN) and HTTP/1.1 elsewhere. The maintainer asked for HTTP/2 on 2026-10-16: PikPak's
//! servers answer each range request only after a few hundred milliseconds, and over HTTP/1.1 every
//! connection sits idle that long between chunks, so the server's congestion window shrinks and
//! each chunk starts slowly again; over HTTP/2 the twelve streams share one connection that is
//! never idle, which is how vget gets its speed. This is the one async corner of the app: the
//! runtime lives on two threads of its own, and callers block on it (`Http::block_on`), so the
//! rest of the app keeps its threads and the Slint event loop (docs/architecture.md, Threads).
//!
//! The flow-control windows matter: hyper's defaults (64 KB) would cap a stream at a few hundred
//! kilobytes per second across the Pacific. They are set the way Go's transport sets them.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::{Body, Frame, Incoming, SizeHint};
use hyper::header::{AUTHORIZATION, LOCATION, USER_AGENT};
use hyper::{Method, Request, Response, StatusCode, Uri};
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::{TokioExecutor, TokioTimer};
use tokio::runtime::{Handle, Runtime};

/// Opening a connection.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// From a request sent to its answer's headers.
pub const RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
/// A connection nobody uses is closed after this.
const IDLE_TIMEOUT: Duration = Duration::from_secs(90);
/// HTTP/2 flow control: what one stream, and the whole connection, may have in flight. Go's
/// transport uses 4 MB per stream; 32 MB on the connection is 190 ms of 70 MB/s with room to spare.
const STREAM_WINDOW: u32 = 4 << 20;
const CONNECTION_WINDOW: u32 = 32 << 20;
/// The largest HTTP/2 frame we accept: fewer frames per chunk.
const FRAME_SIZE: u32 = 1 << 20;
const MOST_REDIRECTS: usize = 10;
/// A piece of a file read and sent at a time (an upload), and how many may wait for the wire.
const UPLOAD_PIECE: usize = 256 * 1024;
const UPLOAD_PIECES_AHEAD: usize = 8;

/// The client, cheap to clone: the connection pool and the runtime's handle. Uploads have a
/// pool of their own, since their requests carry a body read from a file as it goes
/// (`FileBody`) where every other request carries nothing, or a few bytes.
#[derive(Clone)]
pub struct Http {
    client: Client<HttpsConnector<HttpConnector>, Full<Bytes>>,
    uploads: Client<HttpsConnector<HttpConnector>, FileBody>,
    handle: Handle,
}

/// What an upload sends (the design's ninth round): the file at `path`, `size` bytes of it, read
/// as the request goes out. `sent` counts the bytes handed to the connection so far, for the
/// page; `cancel` stops the reading.
pub struct Upload {
    pub path: PathBuf,
    pub size: u64,
    pub cancel: Arc<AtomicBool>,
    pub sent: Arc<AtomicU64>,
}

impl Upload {
    /// The body of one request: a thread reads the file piece by piece as the connection takes
    /// them. The size is known from the start, so the request says Content-Length and never
    /// chunks: a server that refuses chunked uploads (nginx's dav module answers 411) takes it.
    fn body(&self) -> FileBody {
        let (pieces, taken) = tokio::sync::mpsc::channel(UPLOAD_PIECES_AHEAD);
        self.sent.store(0, Ordering::Relaxed);
        if self.size > 0 {
            let (path, size, cancel) = (self.path.clone(), self.size, self.cancel.clone());
            let reading = std::thread::Builder::new().name("upload".into()).spawn(move || {
                use std::io::Read as _;
                let mut file = match std::fs::File::open(&path) {
                    Ok(file) => file,
                    Err(err) => {
                        let _ = pieces.blocking_send(Err(err));
                        return;
                    }
                };
                let mut left = size;
                let mut buffer = vec![0u8; UPLOAD_PIECE];
                while left > 0 {
                    if cancel.load(Ordering::Relaxed) {
                        let _ = pieces.blocking_send(Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "cancelled")));
                        return;
                    }
                    let want = left.min(UPLOAD_PIECE as u64) as usize;
                    let read = match file.read(&mut buffer[..want]) {
                        // The file is shorter than it was when the upload was queued.
                        Ok(0) => Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, format!("the file ended {left} bytes early"))),
                        Ok(read) => Ok(read),
                        Err(err) => Err(err),
                    };
                    let piece = match read {
                        Ok(read) => Bytes::copy_from_slice(&buffer[..read]),
                        Err(err) => {
                            let _ = pieces.blocking_send(Err(err));
                            return;
                        }
                    };
                    left -= piece.len() as u64;
                    // The request was dropped: nobody takes the pieces any more.
                    if pieces.blocking_send(Ok(piece)).is_err() {
                        return;
                    }
                }
            });
            if let Err(err) = reading {
                eprintln!("downloads: cannot start a thread to read the file: {err}");
            }
        }
        FileBody { pieces: taken, left: self.size, sent: self.sent.clone() }
    }
}

/// A request's body read from a file as the connection takes it (`Upload::body`).
pub struct FileBody {
    pieces: tokio::sync::mpsc::Receiver<Result<Bytes, std::io::Error>>,
    left: u64,
    sent: Arc<AtomicU64>,
}

impl Body for FileBody {
    type Data = Bytes;
    type Error = std::io::Error;

    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, std::io::Error>>> {
        let body = self.get_mut();
        if body.left == 0 {
            return Poll::Ready(None);
        }
        match body.pieces.poll_recv(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => Poll::Ready(Some(Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "the file's reader stopped")))),
            Poll::Ready(Some(Err(err))) => Poll::Ready(Some(Err(err))),
            Poll::Ready(Some(Ok(piece))) => {
                body.left = body.left.saturating_sub(piece.len() as u64);
                body.sent.fetch_add(piece.len() as u64, Ordering::Relaxed);
                Poll::Ready(Some(Ok(Frame::data(piece))))
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        self.left == 0
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::with_exact(self.left)
    }
}

/// Why a request got no answer at all. What an answer says is for the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// No connection, or it broke: the host, a refusal, a certificate, a reset.
    Connection(String),
    /// The server did not answer in time.
    Timeout,
    /// The server sent us round in circles, or somewhere we cannot parse.
    Redirect(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Connection(why) => write!(f, "{why}"),
            Failure::Timeout => write!(f, "the server did not answer in time"),
            Failure::Redirect(why) => write!(f, "{why}"),
        }
    }
}

static RUNTIME: OnceLock<Runtime> = OnceLock::new();
static CLIENT: OnceLock<Http> = OnceLock::new();

/// The runtime: two threads named "http", made the first time anything asks.
fn runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("http")
            .enable_all()
            .build()
            .expect("the downloads' tokio runtime")
    })
}

/// The app's one client for downloads: every transfer and every listing shares its pool, so
/// several files from one server travel over the same HTTP/2 connection.
pub fn client() -> &'static Http {
    CLIENT.get_or_init(|| Http::build(false))
}

/// What the app calls itself to a server.
pub fn user_agent() -> String {
    format!("finchgram/{}", crate::update::CURRENT_VERSION)
}

impl Http {
    /// A client; `h2_only` speaks HTTP/2 to a server that was not asked (cleartext, prior
    /// knowledge), which the tests' own server does.
    pub(crate) fn build(h2_only: bool) -> Http {
        let handle = runtime().handle().clone();
        let https = || {
            let mut http = HttpConnector::new();
            http.enforce_http(false);
            http.set_connect_timeout(Some(CONNECT_TIMEOUT));
            http.set_nodelay(true);
            hyper_rustls::HttpsConnectorBuilder::new().with_platform_verifier().https_or_http().enable_http1().enable_http2().wrap_connector(http)
        };
        let mut builder = Client::builder(TokioExecutor::new());
        builder
            // hyper needs to be told what a timer is, or its keep-alives and idle timeouts panic.
            .timer(TokioTimer::new())
            .pool_timer(TokioTimer::new())
            .pool_idle_timeout(IDLE_TIMEOUT)
            .http2_initial_stream_window_size(STREAM_WINDOW)
            .http2_initial_connection_window_size(CONNECTION_WINDOW)
            .http2_max_frame_size(FRAME_SIZE)
            .http2_keep_alive_interval(Some(Duration::from_secs(30)))
            .http2_keep_alive_timeout(Duration::from_secs(15))
            .http2_keep_alive_while_idle(true);
        if h2_only {
            builder.http2_only(true);
        }
        Http { client: builder.build(https()), uploads: builder.build(https()), handle }
    }

    /// The runtime's handle, for a task of the caller's own.
    pub fn handle(&self) -> &Handle {
        &self.handle
    }

    /// Run `future` to its end on this thread, with the runtime's threads doing the I/O.
    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        self.handle.block_on(future)
    }

    /// A request to `url`, with the app's name on it; the caller adds the rest.
    pub fn request(&self, method: Method, url: &str) -> hyper::http::request::Builder {
        Request::builder().method(method).uri(url).header(USER_AGENT, user_agent())
    }

    /// Send `request` and wait for the answer's headers, following redirects: the account's
    /// credentials stay behind when the origin changes, and a redirect from https to http is
    /// refused (`downgrades`). The body is the caller's to read.
    pub async fn send(&self, request: Request<Full<Bytes>>) -> Result<Response<Incoming>, Failure> {
        let (mut parts, body) = request.into_parts();
        let mut hops = 0;
        loop {
            let attempt = Request::from_parts(parts.clone(), body.clone());
            let response = tokio::time::timeout(RESPONSE_TIMEOUT, self.client.request(attempt))
                .await
                .map_err(|_| Failure::Timeout)?
                .map_err(|err| Failure::Connection(describe(&err)))?;
            if !response.status().is_redirection() {
                return Ok(response);
            }
            let Some(location) = response.headers().get(LOCATION).and_then(|value| value.to_str().ok()) else {
                return Ok(response);
            };
            hops += 1;
            if hops > MOST_REDIRECTS {
                return Err(Failure::Redirect("the server redirected too many times".into()));
            }
            let target = resolve(&parts.uri, location)?;
            super::log::line(format!("{} {} redirects to {}://{}/…", parts.uri.host().unwrap_or_default(), response.status().as_u16(), target.scheme_str().unwrap_or_default(), target.host().unwrap_or_default()));
            if downgrades(&parts.uri, &target) {
                return Err(Failure::Redirect(format!(
                    "the server redirected from https to {}, which would send the request in the clear",
                    target.scheme_str().unwrap_or("an address without a scheme")
                )));
            }
            // The credentials were given for one origin: another scheme, host or port does not
            // see them. (The authority alone is the same for https://nas.home and http://nas.home.)
            if !same_origin(&parts.uri, &target) {
                parts.headers.remove(AUTHORIZATION);
            }
            if response.status() == StatusCode::SEE_OTHER && parts.method != Method::GET {
                parts.method = Method::GET;
            }
            parts.uri = target;
        }
    }

    /// Send the request `parts` with `upload` as its body (a PUT), and wait for the answer's
    /// headers, which come once the whole file has gone: no timeout here, the caller watches the
    /// bytes move (webdav.rs). A redirect that keeps the method (301, 302, 307, 308) is followed
    /// with the file read again from its first byte, under `send`'s rules for the credentials
    /// and for https; a 303 is the answer.
    pub async fn send_file(&self, mut parts: hyper::http::request::Parts, upload: &Upload) -> Result<Response<Incoming>, Failure> {
        let mut hops = 0;
        loop {
            let attempt = Request::from_parts(parts.clone(), upload.body());
            let response = self.uploads.request(attempt).await.map_err(|err| Failure::Connection(describe(&err)))?;
            if !response.status().is_redirection() || response.status() == StatusCode::SEE_OTHER {
                return Ok(response);
            }
            let Some(location) = response.headers().get(LOCATION).and_then(|value| value.to_str().ok()) else {
                return Ok(response);
            };
            hops += 1;
            if hops > MOST_REDIRECTS {
                return Err(Failure::Redirect("the server redirected too many times".into()));
            }
            let target = resolve(&parts.uri, location)?;
            super::log::line(format!("{} {} redirects the upload to {}://{}/…", parts.uri.host().unwrap_or_default(), response.status().as_u16(), target.scheme_str().unwrap_or_default(), target.host().unwrap_or_default()));
            if downgrades(&parts.uri, &target) {
                return Err(Failure::Redirect(format!(
                    "the server redirected from https to {}, which would send the file in the clear",
                    target.scheme_str().unwrap_or("an address without a scheme")
                )));
            }
            if !same_origin(&parts.uri, &target) {
                parts.headers.remove(AUTHORIZATION);
            }
            parts.uri = target;
        }
    }
}

/// Where `location` points, from `base`: an address of its own, one from the host's root, or one
/// beside the base's path.
fn resolve(base: &Uri, location: &str) -> Result<Uri, Failure> {
    let bad = |_| Failure::Redirect(format!("the server redirected to {location}, which is not an address"));
    if location.contains("://") {
        return location.parse::<Uri>().map_err(bad);
    }
    let scheme = base.scheme_str().unwrap_or("http");
    let authority = base.authority().map(|authority| authority.as_str()).unwrap_or_default();
    let path = if location.starts_with('/') {
        location.to_string()
    } else {
        let folder = base.path().rsplit_once('/').map(|(folder, _)| folder).unwrap_or_default();
        format!("{folder}/{location}")
    };
    format!("{scheme}://{authority}{path}").parse::<Uri>().map_err(bad)
}

/// Whether going from `from` to `to` leaves https for anything else: what was encrypted, the
/// request's headers among it, would travel in the clear.
fn downgrades(from: &Uri, to: &Uri) -> bool {
    let is_https = |uri: &Uri| uri.scheme_str().is_some_and(|scheme| scheme.eq_ignore_ascii_case("https"));
    is_https(from) && !is_https(to)
}

/// Whether two addresses are of one origin: the same scheme, host and port, the port being the
/// scheme's own (80, 443) when an address names none.
fn same_origin(a: &Uri, b: &Uri) -> bool {
    fn scheme(uri: &Uri) -> Option<String> {
        uri.scheme_str().map(str::to_ascii_lowercase)
    }
    fn port(uri: &Uri) -> Option<u16> {
        uri.port_u16().or(match scheme(uri).as_deref() {
            Some("http") => Some(80),
            Some("https") => Some(443),
            _ => None,
        })
    }
    let host = |uri: &Uri| uri.host().map(str::to_ascii_lowercase);
    scheme(a) == scheme(b) && host(a) == host(b) && port(a) == port(b)
}

/// hyper's errors say little on their own ("client error (Connect)"); what they wrap says why.
fn describe(err: &dyn std::error::Error) -> String {
    let mut words = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        let cause_words = cause.to_string();
        if !words.contains(&cause_words) {
            words.push_str(": ");
            words.push_str(&cause_words);
        }
        source = cause.source();
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;

    /// A server on the loopback that answers one connection per reply, each with its status line
    /// and headers and no body, and gives back the head of every request as it came.
    fn answers(replies: Vec<(&'static str, String)>) -> (u16, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
        let port = listener.local_addr().expect("an address").port();
        let server = std::thread::spawn(move || {
            let mut heads = Vec::new();
            for (status, headers) in replies {
                let (mut socket, _) = listener.accept().expect("a connection");
                let mut head = Vec::new();
                let mut byte = [0u8];
                while !head.ends_with(b"\r\n\r\n") && socket.read(&mut byte).is_ok_and(|n| n == 1) {
                    head.push(byte[0]);
                }
                let _ = write!(socket, "HTTP/1.1 {status}\r\n{headers}Content-Length: 0\r\nConnection: close\r\n\r\n");
                heads.push(String::from_utf8_lossy(&head).to_ascii_lowercase());
            }
            heads
        });
        (port, server)
    }

    fn get(http: &Http, url: &str) -> Response<Incoming> {
        let request = http.request(Method::GET, url).header(AUTHORIZATION, "Basic c2VjcmV0").body(Full::new(Bytes::new())).expect("a request");
        http.block_on(http.send(request)).expect("an answer")
    }

    #[test]
    fn a_redirect_to_another_origin_does_not_carry_the_account() {
        let (target, target_server) = answers(vec![("200 OK", String::new())]);
        let (first, first_server) = answers(vec![("302 Found", format!("Location: http://127.0.0.1:{target}/elsewhere\r\n"))]);
        let response = get(client(), &format!("http://127.0.0.1:{first}/file"));
        assert_eq!(response.status(), StatusCode::OK);
        let first_head = first_server.join().expect("the first server").remove(0);
        let target_head = target_server.join().expect("the target server").remove(0);
        assert!(first_head.contains("authorization: basic c2vjcmv0"), "{first_head}");
        assert!(target_head.starts_with("get /elsewhere ") && !target_head.contains("authorization:"), "{target_head}");
    }

    #[test]
    fn a_redirect_within_the_origin_keeps_the_account() {
        let (port, server) = answers(vec![("302 Found", "Location: /moved\r\n".to_string()), ("200 OK", String::new())]);
        let response = get(client(), &format!("http://127.0.0.1:{port}/file"));
        assert_eq!(response.status(), StatusCode::OK);
        let heads = server.join().expect("the server");
        assert!(heads[1].starts_with("get /moved ") && heads[1].contains("authorization: basic c2vjcmv0"), "{}", heads[1]);
    }

    #[test]
    fn one_origin_is_the_scheme_the_host_and_the_port() {
        let uri = |address: &str| address.parse::<Uri>().expect("an address");
        assert!(same_origin(&uri("https://nas.home/dav"), &uri("https://nas.home:443/other")));
        assert!(same_origin(&uri("http://NAS.home/dav"), &uri("http://nas.home:80/")));
        assert!(!same_origin(&uri("https://nas.home/dav"), &uri("http://nas.home/dav")));
        assert!(!same_origin(&uri("http://nas.home/dav"), &uri("http://nas.home:8080/dav")));
        assert!(!same_origin(&uri("https://nas.home/dav"), &uri("https://cdn.example/dav")));
        assert!(downgrades(&uri("https://nas.home/dav"), &uri("http://nas.home/dav")));
        assert!(!downgrades(&uri("http://nas.home/dav"), &uri("https://nas.home/dav")));
        assert!(!downgrades(&uri("https://nas.home/dav"), &uri("https://cdn.example/x")));
        assert!(!downgrades(&uri("http://nas.home/dav"), &uri("http://cdn.example/x")));
    }
}
