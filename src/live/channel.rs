//! A directory's result as the pages show it (LiveChannel, ui/live.slint): its status, its one
//! action and why (the canvas, round 12), its category and language in the page's words. The
//! thumbnail comes later (thumbnails.rs); the follow mark is the module's to set.

use slint::{Image, SharedString};

use super::directory::{Result_, minutes_since};
use crate::{LiveCategory, LiveChannel, LivePlay, LiveSource, LiveStatus, LiveWhy};

pub fn source_of(id: &str) -> Option<LiveSource> {
    Some(match id {
        "owncast" => LiveSource::Owncast,
        "peertube" => LiveSource::Peertube,
        "icecast" => LiveSource::Icecast,
        "hls" => LiveSource::Hls,
        "telegram" => LiveSource::Telegram,
        "twitch" => LiveSource::Twitch,
        "youtube" => LiveSource::Youtube,
        "kick" => LiveSource::Kick,
        "bilibili" => LiveSource::Bilibili,
        _ => return None,
    })
}

pub fn source_id(source: LiveSource) -> &'static str {
    match source {
        LiveSource::Owncast => "owncast",
        LiveSource::Peertube => "peertube",
        LiveSource::Icecast => "icecast",
        LiveSource::Hls => "hls",
        LiveSource::Telegram => "telegram",
        LiveSource::Twitch => "twitch",
        LiveSource::Youtube => "youtube",
        LiveSource::Kick => "kick",
        LiveSource::Bilibili => "bilibili",
    }
}

pub fn category_of(id: Option<&str>) -> LiveCategory {
    match id.unwrap_or("") {
        "music" => LiveCategory::Music,
        "gaming" => LiveCategory::Gaming,
        "talk" => LiveCategory::Talk,
        "technology" => LiveCategory::Technology,
        "creative" => LiveCategory::Creative,
        "irl" => LiveCategory::Irl,
        "education" => LiveCategory::Education,
        "events" => LiveCategory::Events,
        "news" => LiveCategory::News,
        "sports" => LiveCategory::Sports,
        "other" => LiveCategory::Other,
        _ => LiveCategory::None,
    }
}

pub fn category_id(category: LiveCategory) -> Option<&'static str> {
    Some(match category {
        LiveCategory::Music => "music",
        LiveCategory::Gaming => "gaming",
        LiveCategory::Talk => "talk",
        LiveCategory::Technology => "technology",
        LiveCategory::Creative => "creative",
        LiveCategory::Irl => "irl",
        LiveCategory::Education => "education",
        LiveCategory::Events => "events",
        LiveCategory::News => "news",
        LiveCategory::Sports => "sports",
        LiveCategory::Other => "other",
        LiveCategory::None => return None,
    })
}

pub fn status_of(id: &str) -> LiveStatus {
    match id {
        "live" => LiveStatus::Live,
        "stale" => LiveStatus::Stale,
        "offline" => LiveStatus::Offline,
        _ => LiveStatus::Unknown,
    }
}

/// A language's own name, the way a language menu writes it; the tag itself for one not named here.
pub fn language_name(tag: &str) -> String {
    let base = tag.split('-').next().unwrap_or(tag);
    let name = match base {
        "en" => "English",
        "zh" => "中文",
        "ja" => "日本語",
        "ko" => "한국어",
        "de" => "Deutsch",
        "fr" => "Français",
        "es" => "Español",
        "pt" => "Português",
        "it" => "Italiano",
        "ru" => "Русский",
        "uk" => "Українська",
        "pl" => "Polski",
        "nl" => "Nederlands",
        "sv" => "Svenska",
        "no" | "nb" => "Norsk",
        "da" => "Dansk",
        "fi" => "Suomi",
        "cs" => "Čeština",
        "hu" => "Magyar",
        "ro" => "Română",
        "el" => "Ελληνικά",
        "tr" => "Türkçe",
        "ar" => "العربية",
        "he" => "עברית",
        "fa" => "فارسی",
        "hi" => "हिन्दी",
        "id" => "Bahasa Indonesia",
        "vi" => "Tiếng Việt",
        "th" => "ไทย",
        "ca" => "Català",
        "eu" => "Euskara",
        "gl" => "Galego",
        "eo" => "Esperanto",
        _ => return tag.to_string(),
    };
    name.to_string()
}

/// The avatar's colour, from the id: the same channel always the same.
pub fn hue_of(id: &str) -> i32 {
    let mut hash: u32 = 2166136261;
    for byte in id.bytes() {
        hash = (hash ^ u32::from(byte)).wrapping_mul(16777619);
    }
    (hash % 360) as i32
}

pub fn initial_of(name: &str) -> String {
    name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "·".to_string())
}

fn host_of(url: &str) -> String {
    url::Url::parse(url).ok().and_then(|url| url.host_str().map(str::to_string)).unwrap_or_default()
}

/// What playing it now comes to, in the page's terms: the action and why.
pub fn play_of(source: LiveSource, status: LiveStatus, methods: &[String], availability: &str, reason: Option<&str>) -> (LivePlay, LiveWhy) {
    let native = methods.iter().any(|method| matches!(method.as_str(), "hls" | "dash" | "icecast" | "telegram"));
    if !native {
        if methods.iter().any(|method| method == "official_embed") {
            return (LivePlay::Elsewhere, LiveWhy::OfficialPlayer);
        }
        return (LivePlay::Elsewhere, LiveWhy::OwnSite);
    }
    let reason = reason.unwrap_or("");
    match availability {
        "available" if source == LiveSource::Icecast => (LivePlay::Listen, LiveWhy::None),
        "available" => (LivePlay::Watch, LiveWhy::None),
        "sign_in" => (LivePlay::SignIn, LiveWhy::SignIn),
        "unavailable" if reason.contains("WebRTC") => (LivePlay::Unavailable, LiveWhy::Webrtc),
        "unavailable" if status == LiveStatus::Offline => (LivePlay::Unavailable, LiveWhy::Offline),
        "unavailable" if status == LiveStatus::Unknown => (LivePlay::Unavailable, LiveWhy::Unknown),
        "unavailable" => (LivePlay::Unavailable, LiveWhy::Format),
        // Unconfirmed: live when last seen, or a source the app asks itself (Telegram).
        _ => (LivePlay::TryOpening, LiveWhy::NotConfirmed),
    }
}

/// One result of the directory, as a card; `now` is seconds since 1970.
pub fn from_result(result: &Result_, now: i64) -> Option<LiveChannel> {
    let channel = &result.channel;
    let source = source_of(&channel.source)?;
    let session = result.session.as_ref();
    let status = status_of(result.status.as_deref().or(session.map(|session| session.status.as_str())).unwrap_or("unknown"));
    let (play, why) = play_of(source, status, &result.playback.methods, &result.playback.availability, result.playback.reason.as_deref());
    let viewers = session
        .filter(|_| status == LiveStatus::Live)
        .and_then(|session| session.viewers.as_ref())
        .map_or(-1, |viewers| i32::try_from(viewers.value).unwrap_or(i32::MAX));
    let language = channel.language.as_deref().unwrap_or("");
    let minutes = |time: Option<&str>| minutes_since(time, now).map_or(-1, |minutes| i32::try_from(minutes).unwrap_or(i32::MAX));
    let upcoming_minutes = result.upcoming.as_ref().and_then(|upcoming| chrono::DateTime::parse_from_rfc3339(&upcoming.starts_at).ok()).map_or(-1, |starts| {
        i32::try_from(((starts.timestamp() - now) / 60).max(0)).unwrap_or(i32::MAX)
    });
    Some(LiveChannel {
        id: channel.id.clone().into(),
        source,
        name: channel.name.clone().into(),
        title: session.and_then(|session| session.title.clone()).unwrap_or_default().into(),
        initial: initial_of(&channel.name).into(),
        hue: hue_of(&channel.id),
        picture: Image::default(),
        has_picture: false,
        thumbnail: Image::default(),
        has_thumbnail: session.is_some_and(|session| session.thumbnail.is_some()),
        status,
        observed_minutes: minutes(session.and_then(|session| session.last_confirmed_live_at.as_deref().or(session.observed_at.as_deref()))),
        started_minutes: minutes(session.and_then(|session| session.started_at.as_deref())),
        last_live_minutes: minutes(channel.last_live_at.as_deref()),
        viewers,
        self_reported: session.and_then(|session| session.viewers.as_ref()).is_some_and(|viewers| viewers.kind == "self_reported"),
        audio: source == LiveSource::Icecast,
        category: category_of(channel.category.as_deref()),
        language: if language.is_empty() { SharedString::new() } else { language_name(language).into() },
        language_code: language.split('-').next().unwrap_or("").into(),
        mature: channel.classification == "mature",
        play,
        why,
        following: false,
        on_telegram: false,
        host: host_of(&channel.url).into(),
        upcoming_title: result.upcoming.as_ref().map(|upcoming| upcoming.title.clone()).unwrap_or_default().into(),
        upcoming_minutes,
    })
}

/// The thumbnail's address of a result, when its source gave one.
pub fn thumbnail_of(result: &Result_) -> Option<String> {
    result.session.as_ref()?.thumbnail.as_ref().map(|thumbnail| thumbnail.url.clone()).filter(|url| url.starts_with("https://") || url.starts_with("http://"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(value: serde_json::Value) -> Result_ {
        serde_json::from_value(value).expect("a result")
    }

    #[test]
    fn a_live_peertube_stream_is_watched_here_with_its_count() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-08T03:30:00Z").unwrap().timestamp();
        let card = from_result(
            &result(serde_json::json!({
                "channel": { "id": "fg_ch_1", "name": "harbour cam", "source": "peertube", "url": "https://tube.example.org/video-channels/harbour", "category": "irl", "language": "zh-Hans", "classification": "general" },
                "session": { "id": "fg_se_1", "title": "At night", "started_at": "2026-11-08T01:00:00Z", "status": "live", "observed_at": "2026-11-08T03:25:00Z", "last_confirmed_live_at": "2026-11-08T03:25:00Z", "viewers": { "value": 31, "kind": "exact" }, "thumbnail": { "url": "https://tube.example.org/t.jpg" } },
                "playback": { "methods": ["hls"], "availability": "available", "reason": null }
            })),
            now,
        )
        .expect("a card");
        assert_eq!(card.play, LivePlay::Watch);
        assert_eq!(card.status, LiveStatus::Live);
        assert_eq!(card.viewers, 31);
        assert_eq!(card.started_minutes, 150);
        assert_eq!(card.observed_minutes, 5);
        assert_eq!(card.initial.as_str(), "H");
        assert_eq!(card.language.as_str(), "中文");
        assert_eq!(card.language_code.as_str(), "zh");
        assert_eq!(card.category, LiveCategory::Irl);
        assert_eq!(card.host.as_str(), "tube.example.org");
        assert!(card.has_thumbnail);
    }

    #[test]
    fn each_availability_has_its_action_and_reason() {
        let hls = vec!["hls".to_string()];
        assert_eq!(play_of(LiveSource::Icecast, LiveStatus::Live, &["icecast".to_string()], "available", None), (LivePlay::Listen, LiveWhy::None));
        assert_eq!(play_of(LiveSource::Owncast, LiveStatus::Stale, &hls, "unconfirmed", None), (LivePlay::TryOpening, LiveWhy::NotConfirmed));
        assert_eq!(play_of(LiveSource::Owncast, LiveStatus::Offline, &hls, "unavailable", Some("Not live right now")), (LivePlay::Unavailable, LiveWhy::Offline));
        assert_eq!(play_of(LiveSource::Owncast, LiveStatus::Unknown, &hls, "unavailable", None), (LivePlay::Unavailable, LiveWhy::Unknown));
        assert_eq!(play_of(LiveSource::Peertube, LiveStatus::Live, &hls, "sign_in", None), (LivePlay::SignIn, LiveWhy::SignIn));
        assert_eq!(
            play_of(LiveSource::Telegram, LiveStatus::Live, &["telegram".to_string()], "unavailable", Some("Telegram sends this one over WebRTC: watch it on your phone")),
            (LivePlay::Unavailable, LiveWhy::Webrtc)
        );
        assert_eq!(play_of(LiveSource::Twitch, LiveStatus::Unknown, &["official_embed".to_string()], "unavailable", None), (LivePlay::Elsewhere, LiveWhy::OfficialPlayer));
        assert_eq!(play_of(LiveSource::Kick, LiveStatus::Unknown, &["external".to_string()], "unavailable", None), (LivePlay::Elsewhere, LiveWhy::OwnSite));
    }

    #[test]
    fn names_and_colours_are_the_channels_own() {
        assert_eq!(initial_of("-=Radio X=-"), "R");
        assert_eq!(initial_of("24/7 Jazz"), "2");
        assert_eq!(hue_of("fg_ch_1"), hue_of("fg_ch_1"));
        assert!((0..360).contains(&hue_of("anything")));
        assert_eq!(language_name("pt-BR"), "Português");
        assert_eq!(language_name("xx"), "xx");
    }
}
