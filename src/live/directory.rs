//! The live directory's API (docs/live-directory.md, docs/live-directory.openapi.yaml): FinchGram's
//! own at services.finchgram.com, read with a blocking client on the module's worker thread. Its
//! answers are kept by address with their ETags, so that asking again costs a 304. Nothing about
//! the user goes out: a plain GET, the app's name and version, and the languages chosen in
//! Settings › Live as Accept-Language (what Recommended puts first).

use std::collections::HashMap;
use std::io::Read as _;
use std::time::Duration;

use serde::Deserialize;

/// The directory FinchGram reads: its contract's default server.
pub const BASE: &str = "https://services.finchgram.com/live/api/v1";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const CALL_TIMEOUT: Duration = Duration::from_secs(20);
/// An answer larger than this is refused (the contract's clients' rule).
const LARGEST_ANSWER: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
pub struct Channel {
    pub id: String,
    pub name: String,
    pub source: String,
    #[serde(default)]
    pub source_ref: String,
    pub url: String,
    #[serde(default)]
    pub handle: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub classification: String,
    #[serde(default)]
    pub last_live_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Viewers {
    pub value: i64,
    pub kind: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Thumbnail {
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Session {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    pub status: String,
    #[serde(default)]
    pub observed_at: Option<String>,
    #[serde(default)]
    pub last_confirmed_live_at: Option<String>,
    #[serde(default)]
    pub viewers: Option<Viewers>,
    #[serde(default)]
    pub thumbnail: Option<Thumbnail>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Playback {
    #[serde(default)]
    pub methods: Vec<String>,
    pub availability: String,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Scheduled {
    pub title: String,
    pub starts_at: String,
}

/// One result: a channel, its session if it has one, how it plays.
#[derive(Debug, Clone, Deserialize)]
pub struct Result_ {
    pub channel: Channel,
    /// The channel's status, live or not (added within v1; older answers have only the session's).
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub session: Option<Session>,
    pub playback: Playback,
    #[serde(default)]
    pub upcoming: Option<Scheduled>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Total {
    pub value: Option<i64>,
    pub relation: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Facets {
    #[serde(default)]
    pub source: HashMap<String, i64>,
    #[serde(default)]
    pub category: HashMap<String, i64>,
    #[serde(default)]
    pub language: HashMap<String, i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SearchPage {
    pub total: Total,
    pub next_cursor: Option<String>,
    #[serde(default)]
    pub partial: bool,
    pub items: Vec<Result_>,
    #[serde(default)]
    pub facets: Facets,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SuggestedChannel {
    pub id: String,
    pub name: String,
    pub source: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Filter {
    pub kind: String,
    pub value: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Suggestions {
    #[serde(default)]
    pub queries: Vec<String>,
    #[serde(default)]
    pub channels: Vec<SuggestedChannel>,
    #[serde(default)]
    pub filters: Vec<Filter>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Capabilities {
    #[serde(default)]
    pub viewer_counts: String,
    #[serde(default)]
    pub thumbnails: serde_json::Value,
    #[serde(default)]
    pub playback: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Source {
    pub id: String,
    pub indexing: String,
    pub capabilities: Capabilities,
    pub status: String,
    #[serde(default)]
    pub last_successful_sync_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Sources {
    sources: Vec<Source>,
}

#[derive(Debug, Clone, Deserialize)]
struct Lookup {
    channel: Option<Result_>,
}

/// What a search asks for, as the page's controls set it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Query {
    pub words: String,
    pub mode: &'static str,
    pub sources: Vec<&'static str>,
    pub category: Option<&'static str>,
    pub language: Option<String>,
    pub status: Option<&'static str>,
    pub playback: Option<&'static str>,
    pub mature: bool,
    pub sort: Option<&'static str>,
    pub limit: u32,
    pub cursor: Option<String>,
}

impl Query {
    fn address(&self) -> String {
        let mut url = url::Url::parse(&format!("{BASE}/search")).expect("the base address parses");
        {
            let mut pairs = url.query_pairs_mut();
            if !self.words.trim().is_empty() {
                pairs.append_pair("q", self.words.trim());
            }
            pairs.append_pair("mode", self.mode);
            for source in &self.sources {
                pairs.append_pair("source", source);
            }
            if let Some(category) = self.category {
                pairs.append_pair("category", category);
            }
            if let Some(language) = &self.language {
                pairs.append_pair("language", language);
            }
            if let Some(status) = self.status {
                pairs.append_pair("status", status);
            }
            if let Some(playback) = self.playback {
                pairs.append_pair("playback", playback);
            }
            if self.mature {
                pairs.append_pair("content", "general,mature");
            }
            if let Some(sort) = self.sort {
                pairs.append_pair("sort", sort);
            }
            pairs.append_pair("limit", &self.limit.to_string());
            if let Some(cursor) = &self.cursor {
                pairs.append_pair("cursor", cursor);
            }
        }
        url.into()
    }
}

/// The client: an HTTP agent, the languages to send, and the answers kept with their ETags.
pub struct Client {
    agent: ureq::Agent,
    languages: String,
    kept: HashMap<String, (String, Vec<u8>)>,
}

impl Client {
    pub fn new() -> Client {
        let agent = ureq::Agent::config_builder()
            .user_agent(format!("FinchGram/{}", crate::update::CURRENT_VERSION))
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_global(Some(CALL_TIMEOUT))
            .http_status_as_error(false)
            .build()
            .into();
        Client { agent, languages: String::new(), kept: HashMap::new() }
    }

    /// The languages of Settings › Live, as BCP 47 tags, for Recommended.
    pub fn set_languages(&mut self, languages: &[String]) {
        self.languages = languages.iter().filter(|tag| !tag.is_empty()).take(5).cloned().collect::<Vec<_>>().join(", ");
    }

    fn get(&mut self, address: &str) -> Result<Vec<u8>, String> {
        let mut request = self.agent.get(address).header("Accept", "application/json");
        if !self.languages.is_empty() {
            request = request.header("Accept-Language", &self.languages);
        }
        let key = format!("{address}\n{}", self.languages);
        if let Some((etag, _)) = self.kept.get(&key) {
            request = request.header("If-None-Match", etag);
        }
        let mut response = request.call().map_err(|err| format!("the directory did not answer: {err}"))?;
        let status = response.status().as_u16();
        if status == 304 {
            return self.kept.get(&key).map(|(_, body)| body.clone()).ok_or_else(|| "the directory answered 304 for nothing kept".to_string());
        }
        let etag = response.headers().get("ETag").and_then(|value| value.to_str().ok()).map(str::to_string);
        let mut body = Vec::new();
        response.body_mut().as_reader().take(LARGEST_ANSWER + 1).read_to_end(&mut body).map_err(|err| format!("the directory's answer broke off: {err}"))?;
        if body.len() as u64 > LARGEST_ANSWER {
            return Err("the directory's answer is too large".to_string());
        }
        if status == 429 {
            return Err("the directory is busy: try again in a minute".to_string());
        }
        if !(200..300).contains(&status) {
            let message = serde_json::from_slice::<serde_json::Value>(&body)
                .ok()
                .and_then(|value| value.pointer("/error/message").and_then(|message| message.as_str()).map(str::to_string))
                .unwrap_or_else(|| format!("HTTP {status}"));
            return Err(format!("the directory answered: {message}"));
        }
        if let Some(etag) = etag {
            if self.kept.len() > 200 {
                self.kept.clear();
            }
            self.kept.insert(key, (etag, body.clone()));
        }
        Ok(body)
    }

    fn get_json<T: serde::de::DeserializeOwned>(&mut self, address: &str) -> Result<T, String> {
        let body = self.get(address)?;
        serde_json::from_slice(&body).map_err(|err| format!("the directory's answer is not one FinchGram reads: {err}"))
    }

    pub fn search(&mut self, query: &Query) -> Result<SearchPage, String> {
        self.get_json(&query.address())
    }

    pub fn suggest(&mut self, words: &str) -> Result<Suggestions, String> {
        let mut url = url::Url::parse(&format!("{BASE}/suggest")).expect("the base address parses");
        url.query_pairs_mut().append_pair("q", words.trim());
        self.get_json(url.as_str())
    }

    pub fn channel(&mut self, id: &str) -> Result<Option<Result_>, String> {
        let address = format!("{BASE}/channels/{}", url::form_urlencoded::byte_serialize(id.as_bytes()).collect::<String>());
        match self.get_json::<Result_>(&address) {
            Ok(result) => Ok(Some(result)),
            Err(err) if err.contains("No such channel") => Ok(None),
            Err(err) => Err(err),
        }
    }

    pub fn sources(&mut self) -> Result<Vec<Source>, String> {
        Ok(self.get_json::<Sources>(&format!("{BASE}/sources"))?.sources)
    }

    /// A pasted link's channel, by its host alone or a platform's handle: never the whole link.
    pub fn lookup(&mut self, platform: Option<&str>, handle: Option<&str>, host: Option<&str>) -> Result<Option<Result_>, String> {
        let mut url = url::Url::parse(&format!("{BASE}/channels/lookup")).expect("the base address parses");
        {
            let mut pairs = url.query_pairs_mut();
            if let Some(platform) = platform {
                pairs.append_pair("platform", platform);
            }
            if let Some(handle) = handle {
                pairs.append_pair("handle", handle);
            }
            if let Some(host) = host {
                pairs.append_pair("host", host);
            }
        }
        Ok(self.get_json::<Lookup>(url.as_str())?.channel)
    }
}

/// Minutes from an RFC 3339 time to `now` (seconds since 1970); None for no time.
pub fn minutes_since(time: Option<&str>, now: i64) -> Option<i64> {
    let time = chrono::DateTime::parse_from_rfc3339(time?).ok()?;
    Some(((now - time.timestamp()) / 60).max(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_becomes_the_contracts_address() {
        let query = Query {
            words: " keyboard lab ".into(),
            mode: "channels",
            sources: vec!["owncast", "peertube"],
            category: Some("music"),
            language: Some("zh-Hans".into()),
            status: Some("live"),
            playback: Some("native"),
            mature: true,
            sort: Some("recent"),
            limit: 24,
            cursor: Some("abc".into()),
        };
        assert_eq!(
            query.address(),
            format!(
                "{BASE}/search?q=keyboard+lab&mode=channels&source=owncast&source=peertube&category=music&language=zh-Hans&status=live&playback=native&content=general%2Cmature&sort=recent&limit=24&cursor=abc"
            )
        );
    }

    /// The real directory, read as the app reads it (by hand: it asks services.finchgram.com).
    #[test]
    #[ignore = "asks services.finchgram.com; run with: cargo test live::directory -- --ignored"]
    fn the_real_directory_reads() {
        let mut client = Client::new();
        client.set_languages(&["en".to_string()]);
        let query = Query { mode: "live", limit: 12, ..Query::default() };
        let page = client.search(&query).expect("a page");
        assert!(!page.items.is_empty(), "something is live");
        let now = chrono::Utc::now().timestamp();
        let cards: Vec<_> = page.items.iter().filter_map(|item| crate::live::channel::from_result(item, now)).collect();
        assert_eq!(cards.len(), page.items.len(), "every result makes a card");
        let again = client.search(&query).expect("the same page again, through its ETag");
        assert_eq!(again.items.len(), page.items.len());
        let suggestions = client.suggest("jaz").expect("suggestions");
        assert!(!suggestions.queries.is_empty() || !suggestions.channels.is_empty());
        let sources = client.sources().expect("sources");
        assert!(sources.iter().any(|source| source.id == "owncast" && source.indexing == "enabled"));
        let first = &page.items[0].channel.id;
        assert!(client.channel(first).expect("a channel").is_some());
        assert!(client.channel("fg_ch_nothing").expect("no channel").is_none());
        eprintln!("live: {} of {:?}, first {}", page.items.len(), page.total.value, cards[0].name);
    }

    #[test]
    fn a_page_of_the_contract_reads() {
        let page: SearchPage = serde_json::from_value(serde_json::json!({
            "snapshot": "s_1",
            "generated_at": "2026-11-08T03:29:03Z",
            "total": { "value": 2, "relation": "eq" },
            "next_cursor": null,
            "partial": false,
            "missing_sources": [],
            "items": [{
                "channel": { "id": "fg_ch_1", "name": "Harbour Cam", "source": "peertube", "source_ref": "harbour@tube.example.org", "url": "https://tube.example.org/video-channels/harbour", "handle": null, "category": "irl", "language": "en", "classification": "general", "tags": [], "added_at": null, "last_live_at": null, "a_field_of_later": 1 },
                "session": { "id": "fg_se_1", "title": "At night", "started_at": "2026-11-08T01:00:00Z", "status": "live", "observed_at": "2026-11-08T03:25:00Z", "expires_at": null, "last_confirmed_live_at": "2026-11-08T03:25:00Z", "viewers": { "value": 31, "kind": "exact" }, "thumbnail": null },
                "playback": { "methods": ["hls"], "availability": "available", "reason": null },
                "upcoming": null,
                "attribution": "PeerTube instances"
            }],
            "facets": { "source": { "peertube": 1 }, "category": {}, "language": {}, "approximate": false },
            "sources": [{ "id": "peertube", "status": "ok", "last_successful_sync_at": "2026-11-08T03:25:00Z" }]
        }))
        .expect("the contract's page");
        assert_eq!(page.items[0].session.as_ref().and_then(|s| s.viewers.as_ref()).map(|v| v.value), Some(31));
        assert_eq!(minutes_since(Some("2026-11-08T03:25:00Z"), chrono::DateTime::parse_from_rfc3339("2026-11-08T03:30:30Z").unwrap().timestamp()), Some(5));
    }
}
