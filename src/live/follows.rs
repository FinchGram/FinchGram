//! The channels the user follows in the Live view: the database's (`live_follows`, layout 9),
//! never sent anywhere. The directory is asked about them by id; one it no longer lists stays
//! followed, its status unknown.

use slint::{Image, SharedString};

use super::channel;
use crate::{LiveCategory, LiveChannel, LivePlay, LiveSource, LiveStatus, LiveWhy, database};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Follow {
    pub id: String,
    pub source: String,
    pub name: String,
    pub added: i64,
}

pub fn all() -> Vec<Follow> {
    database::live_follows().into_iter().map(|record| Follow { id: record.id, source: record.source, name: record.name, added: record.added }).collect()
}

pub fn save(follows: &[Follow]) -> Result<(), String> {
    let records: Vec<database::LiveFollowRecord> =
        follows.iter().map(|follow| database::LiveFollowRecord { id: follow.id.clone(), source: follow.source.clone(), name: follow.name.clone(), added: follow.added }).collect();
    database::save_live_follows(&records)
}

/// A followed channel the directory has not told about (yet, or any more): its name as followed,
/// its status unknown.
pub fn placeholder(follow: &Follow) -> LiveChannel {
    let source = channel::source_of(&follow.source).unwrap_or(LiveSource::Owncast);
    LiveChannel {
        id: follow.id.clone().into(),
        source,
        name: follow.name.clone().into(),
        title: SharedString::new(),
        initial: channel::initial_of(&follow.name).into(),
        hue: channel::hue_of(&follow.id),
        picture: Image::default(),
        has_picture: false,
        thumbnail: Image::default(),
        has_thumbnail: false,
        status: LiveStatus::Unknown,
        observed_minutes: -1,
        started_minutes: -1,
        last_live_minutes: -1,
        viewers: -1,
        self_reported: false,
        audio: source == LiveSource::Icecast,
        category: LiveCategory::None,
        language: SharedString::new(),
        language_code: SharedString::new(),
        mature: false,
        play: LivePlay::Unavailable,
        why: LiveWhy::Unknown,
        following: true,
        on_telegram: false,
        host: SharedString::new(),
        upcoming_title: SharedString::new(),
        upcoming_minutes: -1,
    }
}
