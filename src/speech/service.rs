//! The services voice input talks to, named by their API's format (docs/voice-input.md): each
//! format's own address, model and page for keys when the user names none; how an address the user
//! typed or pasted becomes the endpoint; and the connections, HTTP through ureq and WebSockets
//! through tungstenite over rustls with the platform's trust store, all blocking, on the threads
//! src/speech/ starts. Any vendor, relay or server of the user's own that speaks one of the formats
//! can be used: the address is theirs to give.

use std::io::{self, ErrorKind, Read, Write};
use std::net::{IpAddr, TcpStream, ToSocketAddrs};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use rustls_platform_verifier::ConfigVerifierExt as _;
use tungstenite::client::IntoClientRequest as _;
use tungstenite::handshake::HandshakeError;
use tungstenite::http::{HeaderName, HeaderValue, Uri};
use tungstenite::{Message, WebSocket};

/// The connections' formats, as settings.toml names them.
pub const CONNECTION_FORMATS: [&str; 5] = ["openai", "anthropic", "gemini", "dashscope", "doubao"];

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
const WRITE_TIMEOUT: Duration = Duration::from_secs(15);
/// How long a read waits for the service before a session's loop looks at the microphone again.
pub const POLL: Duration = Duration::from_millis(20);

/// The format a connection's API speaks, and with it what the connection can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionFormat {
    /// OpenAI's, and every vendor's, relay's and server's that speaks it: speech (realtime, or an
    /// upload at the end) and text.
    OpenAi,
    /// Anthropic's Messages: text.
    Anthropic,
    /// Google's Gemini: text.
    Gemini,
    /// Alibaba Cloud Model Studio (DashScope): speech (Qwen-ASR, realtime) and text (Qwen, in its
    /// OpenAI-compatible mode), one key for both.
    DashScope,
    /// Volcengine's Doubao speech (Seed-ASR): speech.
    Doubao,
}

impl ConnectionFormat {
    pub fn from_name(name: &str) -> ConnectionFormat {
        match name {
            "anthropic" => ConnectionFormat::Anthropic,
            "gemini" => ConnectionFormat::Gemini,
            "dashscope" => ConnectionFormat::DashScope,
            "doubao" => ConnectionFormat::Doubao,
            _ => ConnectionFormat::OpenAi,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ConnectionFormat::OpenAi => "openai",
            ConnectionFormat::Anthropic => "anthropic",
            ConnectionFormat::Gemini => "gemini",
            ConnectionFormat::DashScope => "dashscope",
            ConnectionFormat::Doubao => "doubao",
        }
    }

    /// The address when the user gives none: the vendor's own.
    pub fn default_url(self) -> &'static str {
        match self {
            ConnectionFormat::OpenAi => "https://api.openai.com/v1",
            ConnectionFormat::Anthropic => "https://api.anthropic.com",
            ConnectionFormat::Gemini => "https://generativelanguage.googleapis.com",
            ConnectionFormat::DashScope => "https://dashscope-intl.aliyuncs.com",
            ConnectionFormat::Doubao => "https://openspeech.bytedance.com",
        }
    }

    /// The vendor, as a new connection to its own address is named.
    pub fn vendor(self) -> &'static str {
        match self {
            ConnectionFormat::OpenAi => "OpenAI",
            ConnectionFormat::Anthropic => "Anthropic",
            ConnectionFormat::Gemini => "Gemini",
            ConnectionFormat::DashScope => "Alibaba Cloud",
            ConnectionFormat::Doubao => "Doubao",
        }
    }

    /// Where the vendor's keys are made.
    pub fn key_page(self) -> &'static str {
        match self {
            ConnectionFormat::OpenAi => "https://platform.openai.com/api-keys",
            ConnectionFormat::Anthropic => "https://platform.claude.com/settings/keys",
            ConnectionFormat::Gemini => "https://aistudio.google.com/apikey",
            ConnectionFormat::DashScope => "https://www.alibabacloud.com/help/en/model-studio/get-api-key",
            ConnectionFormat::Doubao => "https://console.volcengine.com/speech/new/setting/apikeys",
        }
    }

    /// It can hear: speech recognition.
    pub fn hears(self) -> bool {
        matches!(self, ConnectionFormat::OpenAi | ConnectionFormat::DashScope | ConnectionFormat::Doubao)
    }

    /// It can tidy: a text model.
    pub fn writes(self) -> bool {
        !matches!(self, ConnectionFormat::Doubao)
    }

    /// The speech model when the user names none: `upload` for OpenAI's upload.
    pub fn speech_model(self, upload: bool) -> &'static str {
        match self {
            ConnectionFormat::OpenAi if upload => "gpt-transcribe",
            ConnectionFormat::DashScope => "qwen3-asr-flash-realtime",
            // Doubao's "model" is the resource it is billed under, Seed-ASR 2.0 by the hour.
            ConnectionFormat::Doubao => "volc.seedasr.sauc.duration",
            _ => "gpt-live-transcribe",
        }
    }

    /// The text model when the user names none: a small, fast one of each, able enough to put a
    /// misheard word right (the words right come first).
    pub fn text_model(self) -> &'static str {
        match self {
            ConnectionFormat::Anthropic => "claude-haiku-5-5",
            ConnectionFormat::Gemini => "gemini-3.5-flash-lite",
            ConnectionFormat::DashScope => "qwen-flash",
            _ => "gpt-5-mini",
        }
    }
}

/// A new connection's name: the vendor's for its own address, else the host.
pub fn default_label(format: &str, url: &str) -> String {
    let format = ConnectionFormat::from_name(format);
    match Address::parse(url, format.default_url()) {
        Ok(address) if url.trim().is_empty() || Address::parse("", format.default_url()).is_ok_and(|own| own.host == address.host) => format.vendor().to_string(),
        Ok(address) => address.host,
        Err(_) => format.vendor().to_string(),
    }
}

/// How a speech service is spoken to: the protocol under a connection's format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeechFormat {
    /// OpenAI's realtime transcription over a WebSocket: words while the user speaks.
    OpenAiRealtime,
    /// OpenAI's `/audio/transcriptions`: the recording sent once the user is done. The one most
    /// services, relays and servers of one's own speak (Groq, SiliconFlow, whisper servers …).
    OpenAiTranscriptions,
    /// Alibaba Cloud Model Studio's (DashScope's) realtime recognition, Qwen-ASR.
    QwenRealtime,
    /// ByteDance's Doubao (Seed-ASR) streaming recognition on Volcengine, in its binary protocol.
    Doubao,
}

impl SpeechFormat {
    /// The rate of the sound it takes.
    pub fn rate(self) -> u32 {
        match self {
            SpeechFormat::OpenAiRealtime => 24_000,
            _ => 16_000,
        }
    }
}

/// How a text model is spoken to: the protocol under a connection's format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TidyFormat {
    /// Chat Completions: OpenAI's, and the one nearly every vendor, relay and local server speaks
    /// (Model Studio's OpenAI-compatible mode among them).
    OpenAi,
    /// Anthropic's Messages.
    Anthropic,
    /// Google's Gemini (generateContent).
    Gemini,
}

// ---- addresses ---------------------------------------------------------------------------------

/// What is wrong with an address the user typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressProblem {
    /// Not an address: no scheme FinchGram speaks (http, https, ws, wss), or no host.
    Malformed,
    /// A name and password in it: they would go to the server in the clear of the address.
    Credentials,
    /// Plain http to a server on the internet: the key would cross it unencrypted.
    Unencrypted,
}

/// An address, checked: whether it is encrypted, its host and port, and its path without a
/// trailing slash (and without a query: FinchGram adds its own).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Address {
    pub secure: bool,
    pub host: String,
    pub port: u16,
    pub path: String,
}

impl Address {
    /// `text` as typed, or `fallback` when it is empty.
    pub fn parse(text: &str, fallback: &str) -> Result<Address, AddressProblem> {
        let text = text.trim();
        let text = if text.is_empty() { fallback } else { text };
        let uri: Uri = text.parse().map_err(|_| AddressProblem::Malformed)?;
        let secure = match uri.scheme_str().map(str::to_ascii_lowercase).as_deref() {
            Some("https" | "wss") => true,
            Some("http" | "ws") => false,
            _ => return Err(AddressProblem::Malformed),
        };
        let authority = uri.authority().ok_or(AddressProblem::Malformed)?;
        if authority.as_str().contains('@') {
            return Err(AddressProblem::Credentials);
        }
        let host = authority.host().trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
        if host.is_empty() {
            return Err(AddressProblem::Malformed);
        }
        if !secure && !local(&host) {
            return Err(AddressProblem::Unencrypted);
        }
        let port = authority.port_u16().unwrap_or(if secure { 443 } else { 80 });
        let path = uri.path().trim_end_matches('/').to_string();
        Ok(Address { secure, host, port, path })
    }

    /// The address with `path` (from the root) and `query`, for HTTP.
    pub fn http(&self, path: &str, query: &str) -> String {
        self.with_scheme(if self.secure { "https" } else { "http" }, path, query)
    }

    /// The same for a WebSocket.
    pub fn websocket(&self, path: &str, query: &str) -> String {
        self.with_scheme(if self.secure { "wss" } else { "ws" }, path, query)
    }

    fn with_scheme(&self, scheme: &str, path: &str, query: &str) -> String {
        let default_port = if self.secure { 443 } else { 80 };
        let host = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        let port = if self.port == default_port { String::new() } else { format!(":{}", self.port) };
        let query = if query.is_empty() { String::new() } else { format!("?{query}") };
        format!("{scheme}://{host}{port}{path}{query}")
    }

    /// The path without the endpoints a pasted address may end with: what is left is the base.
    pub fn base(&self, endings: &[&str]) -> String {
        let mut path = self.path.clone();
        for ending in endings {
            if let Some(stripped) = path.strip_suffix(ending) {
                path = stripped.trim_end_matches('/').to_string();
                break;
            }
        }
        path
    }

    /// The host with its port when it is not the scheme's own, for the user's eyes.
    pub fn shown(&self) -> String {
        let default_port = if self.secure { 443 } else { 80 };
        if self.port == default_port { self.host.clone() } else { format!("{}:{}", self.host, self.port) }
    }
}

/// This Mac or the local network: plain http may go there (a server of the user's own).
fn local(host: &str) -> bool {
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") || host.ends_with(".lan") {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        Ok(IpAddr::V6(ip)) => ip.is_loopback() || (ip.segments()[0] & 0xfe00) == 0xfc00 || (ip.segments()[0] & 0xffc0) == 0xfe80,
        Err(_) => false,
    }
}

// ---- the endpoints -----------------------------------------------------------------------------

/// What OpenAI's formats' addresses may end with when the user pasted an endpoint whole.
const OPENAI_ENDINGS: [&str; 5] = ["/chat/completions", "/audio/transcriptions", "/realtime", "/models", "/responses"];
/// What Model Studio's addresses may end with: its realtime path, its OpenAI-compatible mode, its
/// own API.
const DASHSCOPE_ENDINGS: [&str; 4] = ["/api-ws/v1/realtime", "/compatible-mode/v1/chat/completions", "/compatible-mode/v1", "/api/v1"];

/// A connection's own: its format, its address (the format's own when empty), its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub format: ConnectionFormat,
    pub address: Address,
    pub label: String,
}

impl Endpoint {
    pub fn new(format: &str, url: &str, label: &str) -> Result<Endpoint, AddressProblem> {
        let format = ConnectionFormat::from_name(format);
        let address = Address::parse(url, format.default_url())?;
        let label = if label.trim().is_empty() { default_label(format.name(), url) } else { label.trim().to_string() };
        Ok(Endpoint { format, address, label })
    }

    /// Whether the address is the vendor's own: a key is needed then.
    pub fn official(&self) -> bool {
        Address::parse("", self.format.default_url()).is_ok_and(|own| own.host == self.address.host)
    }
}

/// A speech service as a profile names it: a connection, how it hears, the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Speech {
    pub format: SpeechFormat,
    pub endpoint: Endpoint,
    pub model: String,
}

impl Speech {
    /// `upload` is for an OpenAI connection: the recording sent at the end. None when the
    /// connection's format cannot hear.
    pub fn new(endpoint: Endpoint, upload: bool, model: &str) -> Option<Speech> {
        let format = match endpoint.format {
            ConnectionFormat::OpenAi if upload => SpeechFormat::OpenAiTranscriptions,
            ConnectionFormat::OpenAi => SpeechFormat::OpenAiRealtime,
            ConnectionFormat::DashScope => SpeechFormat::QwenRealtime,
            ConnectionFormat::Doubao => SpeechFormat::Doubao,
            ConnectionFormat::Anthropic | ConnectionFormat::Gemini => return None,
        };
        let model = if model.trim().is_empty() { endpoint.format.speech_model(upload).to_string() } else { model.trim().to_string() };
        Some(Speech { format, endpoint, model })
    }

    pub fn official(&self) -> bool {
        self.endpoint.official()
    }

    /// Who the user is told answered: the connection's name.
    pub fn shown(&self) -> String {
        self.endpoint.label.clone()
    }

    fn address(&self) -> &Address {
        &self.endpoint.address
    }

    pub fn openai_realtime(&self) -> String {
        self.address().websocket(&format!("{}/realtime", self.address().base(&OPENAI_ENDINGS)), "intent=transcription")
    }

    pub fn openai_transcriptions(&self) -> String {
        self.address().http(&format!("{}/audio/transcriptions", self.address().base(&OPENAI_ENDINGS)), "")
    }

    pub fn qwen_realtime(&self) -> String {
        let base = self.address().base(&DASHSCOPE_ENDINGS);
        self.address().websocket(&format!("{base}/api-ws/v1/realtime"), &format!("model={}", query_value(&self.model)))
    }

    /// Doubao's two ways: `two_pass` words while the user speaks, each sentence heard again for the
    /// final words (Chinese and English); else the sound in, the words at the end (the languages
    /// Doubao names one by one).
    pub fn doubao(&self, two_pass: bool) -> String {
        let base = self.address().base(&["/api/v3/sauc/bigmodel_async", "/api/v3/sauc/bigmodel_nostream", "/api/v3/sauc/bigmodel"]);
        let endpoint = if two_pass { "bigmodel_async" } else { "bigmodel_nostream" };
        self.address().websocket(&format!("{base}/api/v3/sauc/{endpoint}"), "")
    }
}

/// A text model as a profile names it: a connection and the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tidy {
    pub format: TidyFormat,
    pub endpoint: Endpoint,
    pub model: String,
}

impl Tidy {
    /// None when the connection's format cannot write.
    pub fn new(endpoint: Endpoint, model: &str) -> Option<Tidy> {
        let format = match endpoint.format {
            ConnectionFormat::OpenAi | ConnectionFormat::DashScope => TidyFormat::OpenAi,
            ConnectionFormat::Anthropic => TidyFormat::Anthropic,
            ConnectionFormat::Gemini => TidyFormat::Gemini,
            ConnectionFormat::Doubao => return None,
        };
        let model = if model.trim().is_empty() { endpoint.format.text_model().to_string() } else { model.trim().to_string() };
        Some(Tidy { format, endpoint, model })
    }

    pub fn official(&self) -> bool {
        self.endpoint.official()
    }

    fn address(&self) -> &Address {
        &self.endpoint.address
    }

    /// Chat Completions: at the base for OpenAI's format; in Model Studio's OpenAI-compatible mode
    /// for its own.
    pub fn chat_completions(&self) -> String {
        if self.endpoint.format == ConnectionFormat::DashScope {
            let base = self.address().base(&DASHSCOPE_ENDINGS);
            return self.address().http(&format!("{base}/compatible-mode/v1/chat/completions"), "");
        }
        self.address().http(&format!("{}/chat/completions", self.address().base(&OPENAI_ENDINGS)), "")
    }

    /// Anthropic's base has no version in it (its SDKs add /v1): one pasted with it is taken too.
    pub fn anthropic(&self, endpoint: &str) -> String {
        let base = self.address().base(&["/v1/messages", "/v1/models", "/messages", "/v1"]);
        self.address().http(&format!("{base}/v1/{endpoint}"), "")
    }

    pub fn gemini_generate(&self) -> String {
        let base = gemini_base(self.address());
        self.address().http(&format!("{base}/v1beta/models/{}:generateContent", query_value(&self.model)), "")
    }
}

fn gemini_base(address: &Address) -> String {
    match address.path.find("/v1beta") {
        Some(at) => address.path[..at].to_string(),
        None => address.path.trim_end_matches('/').to_string(),
    }
}

/// A model's name as it may go into a URL: only the characters names are made of.
fn query_value(text: &str) -> String {
    text.chars().filter(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | ':' | '/')).collect()
}

// ---- failures ----------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    RefusedKey,
    Quota,
    Network,
    /// The service's own message.
    Service,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub kind: FailureKind,
    pub detail: String,
}

impl Failure {
    pub fn network(err: impl ToString) -> Failure {
        Failure { kind: FailureKind::Network, detail: err.to_string() }
    }

    pub fn service(detail: impl ToString) -> Failure {
        let detail = detail.to_string();
        Failure { kind: FailureKind::Service, detail: if detail.is_empty() { "unknown error".into() } else { detail } }
    }

    /// An HTTP answer that is not the one asked for: 401 and 403 are the key, 429 the credit or
    /// the pace; the message is the service's when its body has one.
    pub fn of_status(status: u16, body: &str) -> Failure {
        let detail = error_message(body).unwrap_or_else(|| format!("HTTP {status}"));
        let kind = match status {
            401 | 403 => FailureKind::RefusedKey,
            429 => FailureKind::Quota,
            _ => FailureKind::Service,
        };
        Failure { kind, detail }
    }
}

/// The message in an error answer, in the shapes the formats give it: OpenAI's and Anthropic's
/// (`error.message`), Google's (the same, with a status), DashScope's (`message`).
pub fn error_message(body: &str) -> Option<String> {
    let answer: serde_json::Value = serde_json::from_str(body).ok()?;
    let message = answer["error"]["message"].as_str().or(answer["message"].as_str()).or(answer["error"].as_str())?;
    (!message.is_empty()).then(|| message.to_string())
}

// ---- HTTP --------------------------------------------------------------------------------------

/// A client for one request: its whole time capped at `timeout`, no redirects followed (a key in
/// a header must not follow one to another host), every status an answer to read.
pub fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .user_agent(format!("finchgram/{}", crate::update::CURRENT_VERSION))
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_global(Some(timeout))
        .max_redirects(0)
        .http_status_as_error(false)
        .build()
        .into()
}

/// What a check of a service and its key found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyAnswer {
    Works,
    Refused,
    Failed(String),
}

impl KeyAnswer {
    fn of_status(status: u16, body: &str) -> KeyAnswer {
        match status {
            200..=299 => KeyAnswer::Works,
            401 | 403 => KeyAnswer::Refused,
            _ => KeyAnswer::Failed(error_message(body).unwrap_or_else(|| format!("HTTP {status}"))),
        }
    }
}

/// A GET with `headers`, its status and body.
pub fn get(url: &str, headers: &[(&str, String)], timeout: Duration) -> Result<(u16, String), String> {
    let mut request = agent(timeout).get(url);
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    let mut response = request.call().map_err(|err| err.to_string())?;
    let status = response.status().as_u16();
    let body = response.body_mut().read_to_string().unwrap_or_default();
    Ok((status, body))
}

/// A POST of `body` (of `content_type`) with `headers`, its status and body.
pub fn post(url: &str, headers: &[(&str, String)], content_type: &str, body: Vec<u8>, timeout: Duration) -> Result<(u16, String), String> {
    let mut request = agent(timeout).post(url).header("Content-Type", content_type);
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    let mut response = request.send(body).map_err(|err| err.to_string())?;
    let status = response.status().as_u16();
    let body = response.body_mut().read_to_string().map_err(|err| err.to_string())?;
    Ok((status, body))
}

/// Whether a connection answers at its address and takes the key (none for a server of one's own
/// that asks for none): a request that costs nothing, the list of models, or for the formats that
/// speak only realtime, the WebSocket's handshake.
pub fn check_connection(endpoint: &Endpoint, key: Option<&str>) -> KeyAnswer {
    let address = &endpoint.address;
    let answer = match endpoint.format {
        ConnectionFormat::OpenAi => get(&address.http(&format!("{}/models", address.base(&OPENAI_ENDINGS)), ""), &bearer(key), Duration::from_secs(20)),
        ConnectionFormat::Anthropic => {
            let tidy = Tidy::new(endpoint.clone(), "").expect("Anthropic writes");
            get(&tidy.anthropic("models"), &super::tidy::anthropic_headers(key, endpoint.official()), Duration::from_secs(20))
        }
        ConnectionFormat::Gemini => {
            get(&address.http(&format!("{}/v1beta/models", gemini_base(address)), ""), &super::tidy::gemini_headers(key, endpoint.official()), Duration::from_secs(20))
        }
        ConnectionFormat::DashScope => {
            let speech = Speech::new(endpoint.clone(), false, "").expect("Model Studio hears");
            return handshake(&speech.qwen_realtime(), &bearer(key));
        }
        ConnectionFormat::Doubao => {
            let speech = Speech::new(endpoint.clone(), false, "").expect("Doubao hears");
            return handshake(&speech.doubao(true), &super::doubao::headers(key.unwrap_or(""), &speech.model));
        }
    };
    match answer {
        Ok((status, body)) => KeyAnswer::of_status(status, &body),
        Err(err) => KeyAnswer::Failed(err),
    }
}

/// `Authorization: Bearer key`, or nothing without a key.
pub fn bearer(key: Option<&str>) -> Vec<(&'static str, String)> {
    key.filter(|key| !key.is_empty()).map(|key| vec![("Authorization", format!("Bearer {key}"))]).unwrap_or_default()
}

fn handshake(url: &str, headers: &[(&str, String)]) -> KeyAnswer {
    match connect(url, headers) {
        Ok(mut socket) => {
            let _ = socket.close(None);
            let _ = socket.flush();
            KeyAnswer::Works
        }
        Err(failure) => match failure.kind {
            FailureKind::RefusedKey => KeyAnswer::Refused,
            _ => KeyAnswer::Failed(failure.detail),
        },
    }
}

// ---- WebSockets --------------------------------------------------------------------------------

/// The connection under a WebSocket: TLS, or plain TCP to a server of one's own.
pub enum Stream {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Stream {
    fn tcp(&self) -> &TcpStream {
        match self {
            Stream::Plain(tcp) => tcp,
            Stream::Tls(tls) => &tls.sock,
        }
    }
}

impl Read for Stream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Stream::Plain(tcp) => tcp.read(buffer),
            Stream::Tls(tls) => tls.read(buffer),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self {
            Stream::Plain(tcp) => tcp.write(buffer),
            Stream::Tls(tls) => tls.write(buffer),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Stream::Plain(tcp) => tcp.flush(),
            Stream::Tls(tls) => tls.flush(),
        }
    }
}

pub type Socket = WebSocket<Stream>;

/// The WebSocket at `url` (ws or wss), opened with `headers`; reads then wait at most POLL, so
/// that a session's loop goes on while nothing comes. A refused handshake says why: 401 and 403
/// are the key.
pub fn connect(url: &str, headers: &[(&str, String)]) -> Result<Socket, Failure> {
    let mut request = url.into_client_request().map_err(Failure::network)?;
    for (name, value) in headers {
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(Failure::network)?;
        let value = HeaderValue::from_str(value)
            .map_err(|_| Failure { kind: FailureKind::RefusedKey, detail: "the key holds characters a header cannot carry".into() })?;
        request.headers_mut().insert(name, value);
    }
    let uri = request.uri().clone();
    let host = uri.host().ok_or_else(|| Failure::network("no host"))?.trim_start_matches('[').trim_end_matches(']').to_string();
    let secure = uri.scheme_str() == Some("wss");
    let port = uri.port_u16().unwrap_or(if secure { 443 } else { 80 });
    let addresses: Vec<_> = (host.as_str(), port).to_socket_addrs().map_err(Failure::network)?.collect();
    let mut tcp = None;
    let mut last_error = String::from("no address");
    for address in &addresses {
        match TcpStream::connect_timeout(address, CONNECT_TIMEOUT) {
            Ok(stream) => {
                tcp = Some(stream);
                break;
            }
            Err(err) => last_error = err.to_string(),
        }
    }
    let tcp = tcp.ok_or_else(|| Failure::network(last_error))?;
    let _ = tcp.set_nodelay(true);
    tcp.set_read_timeout(Some(HANDSHAKE_TIMEOUT)).map_err(Failure::network)?;
    tcp.set_write_timeout(Some(WRITE_TIMEOUT)).map_err(Failure::network)?;
    let stream = if secure {
        let name = rustls::pki_types::ServerName::try_from(host).map_err(Failure::network)?;
        let connection = rustls::ClientConnection::new(tls(), name).map_err(Failure::network)?;
        Stream::Tls(Box::new(rustls::StreamOwned::new(connection, tcp)))
    } else {
        Stream::Plain(tcp)
    };
    match tungstenite::client(request, stream) {
        Ok((socket, _)) => {
            socket.get_ref().tcp().set_read_timeout(Some(POLL)).map_err(Failure::network)?;
            Ok(socket)
        }
        Err(HandshakeError::Failure(tungstenite::Error::Http(response))) => {
            let body = response.body().as_deref().map(String::from_utf8_lossy).unwrap_or_default();
            Err(Failure::of_status(response.status().as_u16(), &body))
        }
        Err(HandshakeError::Failure(err)) => Err(Failure::network(err)),
        Err(HandshakeError::Interrupted(_)) => Err(Failure::network("the service did not answer in time")),
    }
}

/// A read that found nothing yet (POLL went by): the loop goes on.
pub fn nothing_yet(err: &tungstenite::Error) -> bool {
    matches!(err, tungstenite::Error::Io(err) if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut))
}

/// `text` sent as a WebSocket text message.
pub fn send_text(socket: &mut Socket, text: String) -> Result<(), Failure> {
    socket.send(Message::text(text)).map_err(Failure::network)
}

fn tls() -> Arc<rustls::ClientConfig> {
    static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let mut config = rustls::ClientConfig::with_platform_verifier()
                .unwrap_or_else(|_| rustls::ClientConfig::builder().with_root_certificates(rustls::RootCertStore::empty()).with_no_client_auth());
            config.alpn_protocols = vec![b"http/1.1".to_vec()];
            Arc::new(config)
        })
        .clone()
}

/// A fresh random id in the UUID's form (version 4), for the services that want one per
/// connection or event.
pub fn random_id() -> String {
    use ring::rand::SecureRandom as _;
    let mut bytes = [0u8; 16];
    if ring::rand::SystemRandom::new().fill(&mut bytes).is_err() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|time| time.as_nanos()).unwrap_or(0);
        bytes = nanos.to_le_bytes();
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = hex::encode(bytes);
    format!("{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint(format: &str, url: &str) -> Endpoint {
        Endpoint::new(format, url, "").expect("an address")
    }

    #[test]
    fn an_empty_address_is_the_formats_own_and_a_pasted_endpoint_is_taken_as_its_base() {
        let speech = Speech::new(endpoint("openai", ""), false, "").expect("OpenAI hears");
        assert_eq!(speech.format, SpeechFormat::OpenAiRealtime);
        assert_eq!(speech.openai_realtime(), "wss://api.openai.com/v1/realtime?intent=transcription");
        assert_eq!(speech.model, "gpt-live-transcribe");
        assert!(speech.official());
        assert_eq!(speech.shown(), "OpenAI");
        // The same connection, uploading.
        let speech = Speech::new(endpoint("openai", ""), true, "").expect("OpenAI hears");
        assert_eq!((speech.format, speech.model.as_str()), (SpeechFormat::OpenAiTranscriptions, "gpt-transcribe"));
        let speech = Speech::new(endpoint("openai", "https://api.groq.com/openai/v1/"), true, "whisper-large-v3").expect("hears");
        assert_eq!(speech.openai_transcriptions(), "https://api.groq.com/openai/v1/audio/transcriptions");
        assert!(!speech.official());
        assert_eq!(speech.shown(), "api.groq.com");
        // An endpoint pasted whole.
        let speech = Speech::new(endpoint("openai", "https://api.siliconflow.cn/v1/audio/transcriptions"), true, "FunAudioLLM/SenseVoiceSmall").expect("hears");
        assert_eq!(speech.openai_transcriptions(), "https://api.siliconflow.cn/v1/audio/transcriptions");
        // A relay's address, for the live model too.
        let speech = Speech::new(endpoint("openai", "https://relay.example.com/v1"), false, "").expect("hears");
        assert_eq!(speech.openai_realtime(), "wss://relay.example.com/v1/realtime?intent=transcription");
        // A server on this Mac, plainly.
        let speech = Speech::new(endpoint("openai", "http://127.0.0.1:8000/v1"), true, "Systran/faster-whisper-large-v3").expect("hears");
        assert_eq!(speech.openai_transcriptions(), "http://127.0.0.1:8000/v1/audio/transcriptions");
        assert_eq!(speech.shown(), "127.0.0.1");
    }

    #[test]
    fn qwen_and_doubao_take_their_hosts_however_pasted() {
        let speech = Speech::new(endpoint("dashscope", ""), false, "").expect("Model Studio hears");
        assert_eq!(speech.format, SpeechFormat::QwenRealtime);
        assert_eq!(speech.qwen_realtime(), "wss://dashscope-intl.aliyuncs.com/api-ws/v1/realtime?model=qwen3-asr-flash-realtime");
        assert_eq!(speech.shown(), "Alibaba Cloud");
        let speech = Speech::new(endpoint("dashscope", "wss://ws-123.ap-southeast-1.maas.aliyuncs.com/api-ws/v1/realtime?model=x"), false, "qwen3-asr-flash-realtime").expect("hears");
        assert_eq!(speech.qwen_realtime(), "wss://ws-123.ap-southeast-1.maas.aliyuncs.com/api-ws/v1/realtime?model=qwen3-asr-flash-realtime");
        let speech = Speech::new(endpoint("dashscope", "https://dashscope.aliyuncs.com/compatible-mode/v1"), false, "").expect("hears");
        assert_eq!(speech.qwen_realtime(), "wss://dashscope.aliyuncs.com/api-ws/v1/realtime?model=qwen3-asr-flash-realtime");
        let speech = Speech::new(endpoint("doubao", ""), false, "").expect("Doubao hears");
        assert_eq!(speech.doubao(true), "wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_async");
        assert_eq!(speech.doubao(false), "wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_nostream");
        assert_eq!(speech.model, "volc.seedasr.sauc.duration");
        let speech = Speech::new(endpoint("doubao", "wss://openspeech.bytedance.com/api/v3/sauc/bigmodel"), false, "volc.bigasr.sauc.duration").expect("hears");
        assert_eq!(speech.doubao(true), "wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_async");
        // A model's name goes into the address only as a name.
        let speech = Speech::new(endpoint("dashscope", ""), false, "qwen3&evil=1").expect("hears");
        assert!(speech.qwen_realtime().ends_with("?model=qwen3evil1"));
    }

    #[test]
    fn the_text_formats_build_their_endpoints_and_model_studio_tidies_in_its_compatible_mode() {
        let tidy = Tidy::new(endpoint("openai", ""), "").expect("OpenAI writes");
        assert_eq!((tidy.chat_completions().as_str(), tidy.model.as_str()), ("https://api.openai.com/v1/chat/completions", "gpt-5-mini"));
        let tidy = Tidy::new(endpoint("dashscope", ""), "").expect("Model Studio writes");
        assert_eq!((tidy.format, tidy.model.as_str()), (TidyFormat::OpenAi, "qwen-flash"));
        assert_eq!(tidy.chat_completions(), "https://dashscope-intl.aliyuncs.com/compatible-mode/v1/chat/completions");
        let tidy = Tidy::new(endpoint("dashscope", "https://dashscope-intl.aliyuncs.com/compatible-mode/v1"), "qwen-plus").expect("writes");
        assert_eq!(tidy.chat_completions(), "https://dashscope-intl.aliyuncs.com/compatible-mode/v1/chat/completions");
        let tidy = Tidy::new(endpoint("anthropic", ""), "").expect("Anthropic writes");
        assert_eq!((tidy.anthropic("messages").as_str(), tidy.model.as_str()), ("https://api.anthropic.com/v1/messages", "claude-haiku-5-5"));
        // A relay's, given with the version, and a vendor's under a path.
        assert_eq!(Tidy::new(endpoint("anthropic", "https://relay.example.com/v1"), "").expect("writes").anthropic("messages"), "https://relay.example.com/v1/messages");
        assert_eq!(Tidy::new(endpoint("anthropic", "https://api.deepseek.com/anthropic"), "").expect("writes").anthropic("messages"), "https://api.deepseek.com/anthropic/v1/messages");
        let tidy = Tidy::new(endpoint("gemini", ""), "").expect("Gemini writes");
        assert_eq!(tidy.gemini_generate(), "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.5-flash-lite:generateContent");
        let tidy = Tidy::new(endpoint("gemini", "https://generativelanguage.googleapis.com/v1beta/"), "gemini-3.8-flash").expect("writes");
        assert_eq!(tidy.gemini_generate(), "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.8-flash:generateContent");
        // What a format cannot do, it is not asked.
        assert!(Tidy::new(endpoint("doubao", ""), "").is_none());
        assert!(Speech::new(endpoint("anthropic", ""), false, "").is_none() && Speech::new(endpoint("gemini", ""), false, "").is_none());
        assert!(ConnectionFormat::DashScope.hears() && ConnectionFormat::DashScope.writes());
        assert!(ConnectionFormat::Doubao.hears() && !ConnectionFormat::Doubao.writes());
        assert!(!ConnectionFormat::Anthropic.hears() && ConnectionFormat::Anthropic.writes());
    }

    #[test]
    fn a_new_connection_is_named_for_its_vendor_or_its_host() {
        assert_eq!(default_label("dashscope", ""), "Alibaba Cloud");
        assert_eq!(default_label("openai", "https://api.openai.com/v1"), "OpenAI");
        assert_eq!(default_label("openai", "https://relay.example.com/v1"), "relay.example.com");
        assert_eq!(default_label("doubao", "not an address"), "Doubao");
        assert_eq!(Endpoint::new("doubao", "", " 豆包语音 ").map(|endpoint| endpoint.label), Ok("豆包语音".to_string()));
    }

    #[test]
    fn plain_http_goes_only_to_this_mac_and_the_local_network() {
        for local in ["http://localhost:11434/v1", "http://127.0.0.1:8000", "http://192.168.1.20:9000/v1", "http://10.0.0.2", "http://[::1]:8080", "http://studio.local:8000", "ws://172.16.3.4/v1"] {
            assert!(Address::parse(local, "").is_ok(), "{local}");
        }
        assert_eq!(Address::parse("http://api.example.com/v1", ""), Err(AddressProblem::Unencrypted));
        assert_eq!(Address::parse("http://8.8.8.8/v1", ""), Err(AddressProblem::Unencrypted));
        assert_eq!(Address::parse("https://user:pass@relay.example.com/v1", ""), Err(AddressProblem::Credentials));
        assert_eq!(Address::parse("ftp://relay.example.com", ""), Err(AddressProblem::Malformed));
        assert_eq!(Address::parse("relay.example.com/v1", ""), Err(AddressProblem::Malformed));
        assert_eq!(Address::parse("  ", "https://api.openai.com/v1").map(|address| address.host), Ok("api.openai.com".to_string()));
        let address = Address::parse("http://[::1]:8080/v1/", "").expect("IPv6");
        assert_eq!(address.http("/v1/models", ""), "http://[::1]:8080/v1/models");
    }

    #[test]
    fn answers_name_the_key_the_credit_or_the_services_words() {
        let body = r#"{"error":{"message":"Incorrect API key provided: sk-abc.","type":"invalid_request_error","code":"invalid_api_key"}}"#;
        assert_eq!(Failure::of_status(401, body), Failure { kind: FailureKind::RefusedKey, detail: "Incorrect API key provided: sk-abc.".into() });
        assert_eq!(Failure::of_status(429, "").kind, FailureKind::Quota);
        assert_eq!(Failure::of_status(403, "<html>"), Failure { kind: FailureKind::RefusedKey, detail: "HTTP 403".into() });
        assert_eq!(Failure::of_status(500, r#"{"code":"InternalError","message":"busy"}"#).detail, "busy");
        assert_eq!(KeyAnswer::of_status(200, ""), KeyAnswer::Works);
        assert_eq!(KeyAnswer::of_status(401, ""), KeyAnswer::Refused);
        assert_eq!(KeyAnswer::of_status(404, ""), KeyAnswer::Failed("HTTP 404".into()));
    }

    #[test]
    fn random_ids_look_like_uuids() {
        let id = random_id();
        assert_eq!(id.len(), 36);
        assert_eq!(id.chars().nth(14), Some('4'));
        assert_ne!(id, random_id());
    }
}
