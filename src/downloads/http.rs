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
use std::sync::OnceLock;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
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

/// The client, cheap to clone: the connection pool and the runtime's handle.
#[derive(Clone)]
pub struct Http {
    client: Client<HttpsConnector<HttpConnector>, Full<Bytes>>,
    handle: Handle,
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
        let mut http = HttpConnector::new();
        http.enforce_http(false);
        http.set_connect_timeout(Some(CONNECT_TIMEOUT));
        http.set_nodelay(true);
        let https = hyper_rustls::HttpsConnectorBuilder::new().with_platform_verifier().https_or_http().enable_http1().enable_http2().wrap_connector(http);
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
        Http { client: builder.build(https), handle }
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

    /// Send `request` and wait for the answer's headers, following redirects (the account's
    /// credentials stay behind when the host changes). The body is the caller's to read.
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
            if target.authority() != parts.uri.authority() {
                parts.headers.remove(AUTHORIZATION);
            }
            if response.status() == StatusCode::SEE_OTHER && parts.method != Method::GET {
                parts.method = Method::GET;
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
