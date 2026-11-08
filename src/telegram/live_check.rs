//! The live directory's Telegram channels (src/live/; the canvas, round 13): the directory has no
//! Telegram account to ask, so it lists them as status unknown and FinchGram asks Telegram itself,
//! for the channels on screen only, one username at a time with a gap between, and when Telegram
//! limits (FLOOD_WAIT, or TDLib's 429) it waits as long as asked. What it learns stays here; the
//! directory is told nothing.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{Error, Slot, api, errors, send};

/// Between two usernames resolved: resolving many quickly is what Telegram limits.
const GAP: Duration = Duration::from_secs(2);
/// What was learned of a channel holds this long.
const FRESH_FOR: Duration = Duration::from_secs(5 * 60);
/// A limit that does not say how long: wait this long.
const WAIT_UNSAID: Duration = Duration::from_secs(60);

/// What Telegram says of a channel now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seen {
    /// A live story or a live stream is on; `stream`: Telegram serves it as a stream FinchGram plays.
    Live { stream: bool },
    NotLive,
    /// No such public username.
    Missing,
}

type Told = Box<dyn Fn(&str, Seen)>;

#[derive(Default)]
struct Queue {
    waiting: VecDeque<String>,
    asking: bool,
    paused_until: Option<Instant>,
    learned: HashMap<String, (Seen, Instant)>,
    told: Option<Told>,
    slot: Slot,
}

thread_local! {
    static QUEUE: RefCell<Queue> = RefCell::new(Queue::default());
}

/// Ask about these usernames (the channels on screen); `told` hears each answer, by username.
/// What was learned lately is told at once and not asked again.
pub fn check(usernames: Vec<String>, told: impl Fn(&str, Seen) + 'static) {
    let slot = super::shown();
    let mut known = Vec::new();
    QUEUE.with(|queue| {
        let mut queue = queue.borrow_mut();
        if queue.slot != slot {
            *queue = Queue { slot, ..Queue::default() };
        }
        queue.told = Some(Box::new(told));
        queue.waiting.clear();
        for name in usernames {
            let name = name.trim_start_matches('@').to_lowercase();
            match queue.learned.get(&name) {
                Some((seen, at)) if at.elapsed() < FRESH_FOR => known.push((name, *seen)),
                _ => {
                    if !queue.waiting.contains(&name) {
                        queue.waiting.push_back(name);
                    }
                }
            }
        }
    });
    for (name, seen) in known {
        tell(&name, seen);
    }
    pump();
}

fn tell(name: &str, seen: Seen) {
    QUEUE.with(|queue| {
        if let Some(told) = queue.borrow().told.as_ref() {
            told(name, seen);
        }
    });
}

/// How long Telegram asked to wait, when an error is that.
fn wait_asked(code: i32, message: &str) -> Option<Duration> {
    let seconds = errors::wait_seconds(message)
        .and_then(|seconds| u64::try_from(seconds).ok())
        .or_else(|| message.strip_prefix("Too Many Requests: retry after ").and_then(|rest| rest.trim().parse().ok()));
    match seconds {
        Some(seconds) => Some(Duration::from_secs(seconds.max(1))),
        None => (code == 429).then_some(WAIT_UNSAID),
    }
}

/// The next username, unless one is out or Telegram asked to wait.
fn pump() {
    let next = QUEUE.with(|queue| {
        let mut queue = queue.borrow_mut();
        if queue.asking || queue.paused_until.is_some_and(|until| Instant::now() < until) {
            return None;
        }
        let name = queue.waiting.pop_front()?;
        queue.asking = true;
        Some((name, queue.slot))
    });
    let Some((name, slot)) = next else { return };
    super::in_context(slot, || {
        send(json!({ "@type": "searchPublicChat", "username": name }), move |answer| match answer {
            Ok(chat) => ask_chat(slot, name, chat),
            Err(Error::Telegram { code, message }) => {
                if let Some(how_long) = wait_asked(code, &message) {
                    QUEUE.with(|queue| {
                        let mut queue = queue.borrow_mut();
                        queue.waiting.push_front(name);
                        queue.asking = false;
                        queue.paused_until = Some(Instant::now() + how_long);
                    });
                    slint::Timer::single_shot(how_long, pump);
                    return;
                }
                done(&name, if code == 400 { Seen::Missing } else { Seen::NotLive });
            }
            Err(Error::Stopped) => QUEUE.with(|queue| queue.borrow_mut().asking = false),
        });
    });
}

/// A chat found: its live story, else its live stream (listed as live.rs lists one).
fn ask_chat(slot: Slot, name: String, chat: Value) {
    let Some(chat_id) = chat.get("id").and_then(Value::as_i64) else { return done(&name, Seen::Missing) };
    let in_channel = chat.pointer("/type/is_channel").and_then(Value::as_bool).unwrap_or(false);
    let call_id = chat.pointer("/video_chat/group_call_id").and_then(Value::as_i64).filter(|id| *id != 0);
    super::in_context(slot, || {
        send(json!({ "@type": "getChatActiveStories", "chat_id": chat_id }), move |answer| {
            let live_story = answer.ok().and_then(|value| serde_json::from_value::<api::ChatActiveStories>(value).ok()).and_then(|active| {
                active.stories.iter().find(|story| story.is_live).map(|story| story.story_id)
            });
            if let Some(story_id) = live_story {
                // Whether Telegram serves it as a stream is the story's: an RTMP live story plays here.
                return super::in_context(slot, || {
                    send(json!({ "@type": "getStory", "story_poster_chat_id": chat_id, "story_id": story_id, "only_local": false }), move |answer| {
                        let stream = answer.ok().and_then(|value| serde_json::from_value::<api::Story>(value).ok()).is_none_or(|story| {
                            !matches!(story.content, api::StoryContent::Live { is_rtmp_stream: false, .. })
                        });
                        done(&name, Seen::Live { stream });
                    });
                });
            }
            let Some(call_id) = call_id else { return done(&name, Seen::NotLive) };
            super::in_context(slot, || {
                send(json!({ "@type": "getGroupCall", "group_call_id": call_id }), move |answer| {
                    let seen = match answer.ok().and_then(|value| serde_json::from_value::<api::GroupCall>(value).ok()) {
                        Some(call) if call.is_active && (in_channel || call.is_rtmp_stream) => Seen::Live { stream: true },
                        _ => Seen::NotLive,
                    };
                    done(&name, seen);
                });
            });
        });
    });
}

fn done(name: &str, seen: Seen) {
    QUEUE.with(|queue| {
        queue.borrow_mut().learned.insert(name.to_string(), (seen, Instant::now()));
    });
    tell(name, seen);
    slint::Timer::single_shot(GAP, || {
        QUEUE.with(|queue| queue.borrow_mut().asking = false);
        pump();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telegrams_limits_say_how_long() {
        assert_eq!(wait_asked(420, "FLOOD_WAIT_35"), Some(Duration::from_secs(35)));
        assert_eq!(wait_asked(429, "Too Many Requests: retry after 12"), Some(Duration::from_secs(12)));
        assert_eq!(wait_asked(429, "Too Many Requests"), Some(WAIT_UNSAID));
        assert_eq!(wait_asked(400, "USERNAME_NOT_OCCUPIED"), None);
    }
}
