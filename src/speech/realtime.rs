//! The two realtime formats that speak in JSON events over a WebSocket: OpenAI's realtime
//! transcription and Alibaba Cloud's Qwen-ASR (DashScope), which follows OpenAI's shape with its
//! own settings and its own live words.
//!
//! OpenAI's (`…/realtime?intent=transcription`): a `session.update` with the model, the language,
//! a sentence of context, and no turn detection (the user says when they are done), then the
//! sound in pieces of 100 ms while the words come back as deltas; the user done, a commit closes
//! the turn and its whole transcript comes (`…completed`).
//!
//! Qwen's (`…/api-ws/v1/realtime?model=…`): its `session.update` names the sound's format and the
//! language, and leaves the server to find where sentences end (in its manual mode it would hear
//! nothing until the commit); each sentence comes as text the server is sure of and a draft after
//! it (`…text`, text + stash), then whole (`…completed`). The user done, `session.finish` has the
//! last sentence finished, and `session.finished` ends it.

use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tungstenite::Message;

use super::audio;
use super::dictation::{Input, Options, Report, Sound, Stopped, english_name, iso_639_1};
use super::join_words;
use super::service::{self, Failure, FailureKind, SpeechFormat};

/// How long the service may take to have the session's settings.
const SETTINGS_WAIT: Duration = Duration::from_secs(15);
/// How long the last words may take once the user is done.
const LAST_WORDS_WAIT: Duration = Duration::from_secs(15);
/// A file's sound waits this long at most for the next input before the loop reads again.
const FILE_POLL: Duration = Duration::from_millis(5);

/// The session: the sound out, the words in, until the user is done and the last words came.
pub fn listen(options: &Options, inputs: &Receiver<Input>, report: &dyn Fn(Report)) -> Result<String, Stopped> {
    let qwen = options.speech.format == SpeechFormat::QwenRealtime;
    let (url, settings) = if qwen {
        (options.speech.qwen_realtime(), qwen_settings(options))
    } else {
        (options.speech.openai_realtime(), openai_settings(options))
    };
    let mut socket = service::connect(&url, &service::bearer(options.speech_key.as_deref()))?;
    service::send_text(&mut socket, with_id(settings, qwen).to_string())?;
    let opened = Instant::now();
    let mut ready = false;
    let mut sound = Sound::new(inputs, options.speech.format.rate());
    // 100 ms a piece.
    let piece = sound.rate() as usize / 10;
    let mut sent = 0;
    let mut transcript = Transcript::default();
    loop {
        if let Err(stopped) = if options.recorded && sound.waiting.is_empty() && sound.finished.is_none() { sound.wait(FILE_POLL) } else { sound.take() } {
            let _ = socket.close(None);
            return Err(stopped);
        }
        if ready {
            while let Some(piece) = sound.piece(piece) {
                sent += piece.len();
                let event = json!({ "type": "input_audio_buffer.append", "audio": audio::encode(&piece) });
                service::send_text(&mut socket, with_id(event, qwen).to_string())?;
            }
            if sound.finished.is_some() && !transcript.end_sent {
                // Less than 100 ms of sound is no turn.
                if sent < sound.rate() as usize / 10 {
                    let _ = socket.close(None);
                    return Ok(transcript.text());
                }
                let end = if qwen { json!({ "type": "session.finish" }) } else { json!({ "type": "input_audio_buffer.commit" }) };
                service::send_text(&mut socket, with_id(end, qwen).to_string())?;
                transcript.end_sent = true;
            }
        }
        match socket.read() {
            Ok(Message::Text(text)) => {
                let event: Value = serde_json::from_str(text.as_str()).unwrap_or(Value::Null);
                match transcript.take(&event) {
                    Event::Ready => {
                        if !ready {
                            ready = true;
                            report(Report::Ready);
                        }
                    }
                    Event::Words => report(Report::Words(transcript.text())),
                    Event::Other => {}
                    Event::Finished => {
                        let _ = socket.close(None);
                        return Ok(transcript.final_text());
                    }
                    Event::NoTurn => {
                        let _ = socket.close(None);
                        return Ok(transcript.text());
                    }
                    Event::Refused(failure) => {
                        let _ = socket.close(None);
                        return Err(Stopped::Failed(failure));
                    }
                }
            }
            Ok(Message::Close(frame)) => {
                if sound.finished.is_some() {
                    return Ok(transcript.final_text());
                }
                let reason = frame.map(|frame| frame.reason.to_string()).filter(|reason| !reason.is_empty());
                return Err(Stopped::Failed(Failure::network(reason.unwrap_or_else(|| "the service closed the connection".into()))));
            }
            Ok(_) => {}
            Err(err) if service::nothing_yet(&err) => {}
            Err(err) => return Err(Stopped::Failed(Failure::network(err))),
        }
        if transcript.finished() || sound.finished.is_some_and(|at| at.elapsed() > LAST_WORDS_WAIT) {
            let _ = socket.close(None);
            return Ok(transcript.final_text());
        }
        if !ready && opened.elapsed() > SETTINGS_WAIT {
            return Err(Stopped::Failed(Failure::network("the service did not take the session's settings in time")));
        }
    }
}

/// Qwen wants an id on every event of the client's.
fn with_id(mut event: Value, qwen: bool) -> Value {
    if qwen {
        event["event_id"] = json!(format!("event_{}", service::random_id().replace('-', "")));
    }
    event
}

/// A sentence of context for the model: what is being said, that words of other languages may
/// come in, to be written as said, and (for the models that take no keywords) the words to know.
fn context(language: &str, keywords: &[String]) -> String {
    let mut context = match english_name(language) {
        Some(name) => format!(
            "A person dictating a chat message, mainly in {name}. Words or phrases in other languages may be mixed in: write them as spoken, in their own script."
        ),
        None => "A person dictating a chat message. More than one language may be spoken, even within a sentence: write each as spoken, in its own script.".to_string(),
    };
    if !keywords.is_empty() {
        context.push_str(&format!(" Names and terms that may come up: {}.", keywords.join(", ")));
    }
    context
}

/// OpenAI's models of 2026 take keywords and a list of languages; the older ones neither.
fn newer_model(model: &str) -> bool {
    model.starts_with("gpt-live") || model.starts_with("gpt-transcribe")
}

/// OpenAI's session: the model, the sound's format, the main language as the one hint (the script
/// with it: "zh-cn" or "zh-tw"), the context, and no turn detection.
fn openai_settings(options: &Options) -> Value {
    let newer = newer_model(&options.speech.model);
    let in_context: &[String] = if newer { &[] } else { &options.keywords };
    let mut transcription = json!({ "model": options.speech.model, "prompt": context(&options.language, in_context) });
    // How long the live model waits to be sure of a word; only it takes the setting. The words
    // right come first (the maintainer's word of 2026-10-26): "xhigh", the most context, though
    // the live words show later; the last ones come when the user is done either way.
    if options.speech.model.starts_with("gpt-live") {
        let delay = if options.delay.is_empty() { "xhigh" } else { options.delay.as_str() };
        transcription["delay"] = json!(delay);
    }
    if newer && !options.keywords.is_empty() {
        transcription["keywords"] = json!(options.keywords);
    }
    // The models of 2026 take a list of languages, with the script ("zh-cn", "zh-tw"); the older
    // ones one language, by ISO 639-1. Never both: OpenAI refuses that.
    if let Some(iso) = iso_639_1(&options.language) {
        if newer {
            transcription["languages"] = json!([options.language]);
        } else {
            transcription["language"] = json!(iso);
        }
    }
    json!({
        "type": "session.update",
        "session": {
            "type": "transcription",
            "audio": {
                "input": {
                    "format": { "type": "audio/pcm", "rate": options.speech.format.rate() },
                    "transcription": transcription,
                    "turn_detection": null,
                },
            },
        },
    })
}

/// Qwen's session: 16 kHz PCM, the language (Qwen writes Chinese one way; the tidying sets the
/// script), and the server's own detection of where sentences end, with time to think between
/// them.
fn qwen_settings(options: &Options) -> Value {
    let mut transcription = json!({});
    if let Some(language) = iso_639_1(&options.language) {
        transcription["language"] = json!(language);
    }
    // The words to know as context, which Qwen biases its hearing toward.
    if !options.keywords.is_empty() {
        transcription["corpus"] = json!({ "text": options.keywords.join(", ") });
    }
    json!({
        "type": "session.update",
        "session": {
            "input_audio_format": "pcm",
            "sample_rate": options.speech.format.rate(),
            "input_audio_transcription": transcription,
            "turn_detection": { "type": "server_vad", "silence_duration_ms": 1200 },
        },
    })
}

/// What an event of the session changes.
#[derive(Debug, PartialEq, Eq)]
enum Event {
    Ready,
    Words,
    Other,
    /// The service said the session is over (Qwen's `session.finished`).
    Finished,
    /// The commit was refused, too little sound: there is no turn, and nothing more comes.
    NoTurn,
    Refused(Failure),
}

/// The words of the session's turns, in the order they began.
#[derive(Default)]
struct Transcript {
    turns: Vec<Turn>,
    /// The turn the commit made, once OpenAI said which.
    committed: Option<String>,
    /// The commit (OpenAI) or the finish (Qwen) went out.
    end_sent: bool,
    /// A turn's whole transcript came after the end went out.
    completed_since_end: bool,
}

struct Turn {
    id: String,
    words: String,
    complete: bool,
}

impl Transcript {
    fn turn(&mut self, id: &str) -> &mut Turn {
        let index = match self.turns.iter().position(|turn| turn.id == id) {
            Some(index) => index,
            None => {
                self.turns.push(Turn { id: id.to_string(), words: String::new(), complete: false });
                self.turns.len() - 1
            }
        };
        &mut self.turns[index]
    }

    fn take(&mut self, event: &Value) -> Event {
        let item = event["item_id"].as_str().unwrap_or("");
        match event["type"].as_str().unwrap_or("") {
            "session.updated" | "transcription_session.updated" => Event::Ready,
            // OpenAI's live words: pieces to add.
            "conversation.item.input_audio_transcription.delta" => {
                let turn = self.turn(item);
                if !turn.complete {
                    turn.words.push_str(event["delta"].as_str().unwrap_or(""));
                }
                Event::Words
            }
            // Qwen's: the sentence so far, sure and draft, in place of what was there.
            "conversation.item.input_audio_transcription.text" => {
                let turn = self.turn(item);
                if !turn.complete {
                    turn.words = format!("{}{}", event["text"].as_str().unwrap_or(""), event["stash"].as_str().unwrap_or(""));
                }
                Event::Words
            }
            "conversation.item.input_audio_transcription.completed" => {
                let turn = self.turn(item);
                turn.words = event["transcript"].as_str().unwrap_or("").to_string();
                turn.complete = true;
                if self.end_sent {
                    self.completed_since_end = true;
                }
                Event::Words
            }
            "conversation.item.input_audio_transcription.failed" => {
                self.turn(item).complete = true;
                if self.end_sent {
                    self.completed_since_end = true;
                }
                if self.text().is_empty() { Event::Refused(failure_of(&event["error"])) } else { Event::Words }
            }
            "input_audio_buffer.committed" => {
                self.turn(item);
                if self.end_sent {
                    self.committed = Some(item.to_string());
                }
                Event::Other
            }
            "session.finished" => Event::Finished,
            "error" => {
                if event["error"]["code"].as_str() == Some("input_audio_buffer_commit_empty") {
                    Event::NoTurn
                } else {
                    Event::Refused(failure_of(&event["error"]))
                }
            }
            _ => Event::Other,
        }
    }

    /// The commit went out and the transcript of its turn came (of a turn, after the commit, when
    /// OpenAI did not say which turn the commit made). Qwen's end is its `session.finished`.
    fn finished(&self) -> bool {
        if !self.end_sent {
            return false;
        }
        match &self.committed {
            Some(id) => self.turns.iter().any(|turn| turn.id == *id && turn.complete),
            None => self.completed_since_end && self.turns.iter().all(|turn| turn.complete || turn.words.is_empty()),
        }
    }

    /// Every turn's words so far, as the strip shows them.
    fn text(&self) -> String {
        join_turns(self.turns.iter())
    }

    /// The words for the composer: the turns whose whole transcript came. A dictation of OpenAI's
    /// is one turn (one commit, no turn detection), so live words left under another turn are the
    /// same sound again, and are not repeated; with no transcript at all, the live words are all
    /// there is. Qwen's turns are its sentences, each completed in its turn.
    fn final_text(&self) -> String {
        if self.turns.iter().any(|turn| turn.complete) { join_turns(self.turns.iter().filter(|turn| turn.complete)) } else { self.text() }
    }
}

fn join_turns<'a>(turns: impl Iterator<Item = &'a Turn>) -> String {
    turns.fold(String::new(), |text, turn| join_words(&text, turn.words.trim()))
}

/// An error object of the session's, as the user is told it.
fn failure_of(error: &Value) -> Failure {
    let code = error["code"].as_str().unwrap_or("");
    let message = error["message"].as_str().filter(|message| !message.is_empty()).unwrap_or(code).to_string();
    let kind = if code == "invalid_api_key" || code.contains("InvalidApiKey") || code.contains("unauthorized") {
        FailureKind::RefusedKey
    } else if code.contains("quota") || code.contains("rate_limit") || code.contains("Throttling") || code.contains("Arrearage") {
        FailureKind::Quota
    } else {
        FailureKind::Service
    };
    Failure { kind, detail: if message.is_empty() { "unknown error".into() } else { message } }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::speech::service::{Endpoint, Speech};

    /// "openai-realtime" or "qwen-realtime", at the vendor's own address.
    fn options(format: &str, model: &str, language: &str) -> Options {
        let connection = if format == "qwen-realtime" { "dashscope" } else { "openai" };
        Options {
            speech: Speech::new(Endpoint::new(connection, "", "").expect("the default address"), false, model).expect("it hears"),
            speech_key: Some("sk-test".into()),
            language: language.into(),
            keywords: Vec::new(),
            delay: String::new(),
            tidy: None,
            recorded: false,
        }
    }

    fn event(text: &str) -> Value {
        serde_json::from_str(text).expect("an event")
    }

    #[test]
    fn openais_session_asks_for_the_live_model_the_main_language_and_no_turn_detection() {
        let settings = openai_settings(&options("openai-realtime", "", "zh-cn"));
        assert_eq!(settings["type"], "session.update");
        assert_eq!(settings["session"]["type"], "transcription");
        let input = &settings["session"]["audio"]["input"];
        assert_eq!(input["format"], json!({ "type": "audio/pcm", "rate": 24000 }));
        let transcription = &input["transcription"];
        assert_eq!((transcription["model"].as_str(), transcription["delay"].as_str()), (Some("gpt-live-transcribe"), Some("xhigh")));
        assert_eq!(transcription["languages"], json!(["zh-cn"]));
        assert!(transcription["prompt"].as_str().is_some_and(|prompt| prompt.contains("Simplified") && prompt.contains("other languages")));
        assert!(input["turn_detection"].is_null() && input.as_object().expect("an object").contains_key("turn_detection"));
        // Any language: no hint; another model: no delay; a delay set by hand is kept.
        let settings = openai_settings(&options("openai-realtime", "gpt-transcribe", "auto"));
        let transcription = &settings["session"]["audio"]["input"]["transcription"];
        assert!(transcription.get("languages").is_none() && transcription.get("delay").is_none());
        // An older model takes one language by ISO 639-1, and the words to know in its context.
        let mut older = options("openai-realtime", "gpt-4o-transcribe", "zh-tw");
        older.keywords = vec!["FinchGram".into(), "陈思远".into()];
        let settings = openai_settings(&older);
        let transcription = &settings["session"]["audio"]["input"]["transcription"];
        assert_eq!((transcription["language"].as_str(), transcription.get("languages")), (Some("zh"), None));
        assert!(transcription.get("keywords").is_none() && transcription["prompt"].as_str().is_some_and(|prompt| prompt.ends_with("FinchGram, 陈思远.")));
        // The newer ones take them as keywords.
        let mut newer = options("openai-realtime", "", "en");
        newer.keywords = vec!["FinchGram".into()];
        let settings = openai_settings(&newer);
        assert_eq!(settings["session"]["audio"]["input"]["transcription"]["keywords"], json!(["FinchGram"]));
        let mut chosen = options("openai-realtime", "", "en");
        chosen.delay = "medium".into();
        assert_eq!(openai_settings(&chosen)["session"]["audio"]["input"]["transcription"]["delay"], "medium");
    }

    #[test]
    fn qwens_session_names_the_sound_the_language_and_the_servers_sentences() {
        let settings = qwen_settings(&options("qwen-realtime", "", "zh-tw"));
        assert_eq!(
            settings,
            json!({
                "type": "session.update",
                "session": {
                    "input_audio_format": "pcm",
                    "sample_rate": 16000,
                    "input_audio_transcription": { "language": "zh" },
                    "turn_detection": { "type": "server_vad", "silence_duration_ms": 1200 },
                },
            })
        );
        assert_eq!(qwen_settings(&options("qwen-realtime", "", "auto"))["session"]["input_audio_transcription"], json!({}));
        let mut biased = options("qwen-realtime", "", "zh-cn");
        biased.keywords = vec!["FinchGram".into(), "陈思远".into()];
        assert_eq!(qwen_settings(&biased)["session"]["input_audio_transcription"]["corpus"], json!({ "text": "FinchGram, 陈思远" }));
        let event = with_id(json!({ "type": "session.finish" }), true);
        assert!(event["event_id"].as_str().is_some_and(|id| id.starts_with("event_") && id.len() > 20));
        assert!(with_id(json!({ "type": "x" }), false).get("event_id").is_none());
    }

    #[test]
    fn openais_words_grow_while_the_user_speaks_and_the_turn_ends_with_its_transcript() {
        let mut transcript = Transcript::default();
        assert_eq!(transcript.take(&event(r#"{"type":"session.created"}"#)), Event::Other);
        assert_eq!(transcript.take(&event(r#"{"type":"session.updated"}"#)), Event::Ready);
        let delta = |words: &str| event(&format!(r#"{{"type":"conversation.item.input_audio_transcription.delta","item_id":"item_1","content_index":0,"delta":"{words}"}}"#));
        assert_eq!(transcript.take(&delta("明天下午")), Event::Words);
        assert_eq!(transcript.take(&delta("三点见")), Event::Words);
        assert_eq!(transcript.text(), "明天下午三点见");
        assert!(!transcript.finished());
        transcript.end_sent = true;
        assert_eq!(transcript.take(&event(r#"{"type":"input_audio_buffer.committed","item_id":"item_1","previous_item_id":null}"#)), Event::Other);
        assert!(!transcript.finished());
        let completed = r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"item_1","content_index":0,"transcript":"明天下午三点见。"}"#;
        assert_eq!(transcript.take(&event(completed)), Event::Words);
        assert_eq!(transcript.text(), "明天下午三点见。");
        assert!(transcript.finished());
        // A delta after the transcript changes nothing.
        transcript.take(&delta("又"));
        assert_eq!(transcript.text(), "明天下午三点见。");
    }

    #[test]
    fn live_words_left_under_another_turn_are_not_said_twice() {
        let mut transcript = Transcript::default();
        transcript.take(&event(r#"{"type":"conversation.item.input_audio_transcription.delta","item_id":"live","delta":"see you at"}"#));
        transcript.end_sent = true;
        transcript.take(&event(r#"{"type":"input_audio_buffer.committed","item_id":"turn"}"#));
        assert!(!transcript.finished());
        transcript.take(&event(r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"turn","transcript":"See you at three."}"#));
        assert!(transcript.finished());
        assert_eq!(transcript.final_text(), "See you at three.");
        // With no transcript at all, the live words are what there is.
        let mut transcript = Transcript::default();
        transcript.take(&event(r#"{"type":"conversation.item.input_audio_transcription.delta","item_id":"live","delta":"hello"}"#));
        assert_eq!(transcript.final_text(), "hello");
    }

    #[test]
    fn qwens_sentences_show_sure_and_draft_and_end_with_the_session() {
        let mut transcript = Transcript::default();
        let text = |item: &str, sure: &str, draft: &str| {
            event(&format!(r#"{{"type":"conversation.item.input_audio_transcription.text","item_id":"{item}","language":"zh","text":"{sure}","stash":"{draft}"}}"#))
        };
        assert_eq!(transcript.take(&text("a", "", "今天")), Event::Words);
        assert_eq!(transcript.take(&text("a", "今天", "天气不错")), Event::Words);
        assert_eq!(transcript.text(), "今天天气不错");
        transcript.take(&event(r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"a","transcript":"今天天气不错，"}"#));
        transcript.take(&text("b", "", "我们去公园"));
        assert_eq!(transcript.text(), "今天天气不错，我们去公园");
        transcript.end_sent = true;
        assert!(!transcript.finished());
        transcript.take(&event(r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"b","transcript":"我们去公园吧。"}"#));
        assert!(transcript.finished());
        assert_eq!(transcript.final_text(), "今天天气不错，我们去公园吧。");
        assert_eq!(transcript.take(&event(r#"{"type":"session.finished","event_id":"e"}"#)), Event::Finished);
    }

    #[test]
    fn errors_say_what_went_wrong_and_too_little_sound_is_no_turn() {
        let mut transcript = Transcript::default();
        let empty = r#"{"type":"error","error":{"type":"invalid_request_error","code":"input_audio_buffer_commit_empty","message":"buffer too small"}}"#;
        assert_eq!(transcript.take(&event(empty)), Event::NoTurn);
        let wrong = r#"{"type":"error","error":{"type":"invalid_request_error","code":"unknown_parameter","message":"Unknown parameter: 'session.audio.input.transcription.delay'."}}"#;
        assert_eq!(
            transcript.take(&event(wrong)),
            Event::Refused(Failure { kind: FailureKind::Service, detail: "Unknown parameter: 'session.audio.input.transcription.delay'.".into() })
        );
        let quota = r#"{"type":"error","error":{"code":"insufficient_quota","message":"You exceeded your current quota."}}"#;
        assert_eq!(transcript.take(&event(quota)), Event::Refused(Failure { kind: FailureKind::Quota, detail: "You exceeded your current quota.".into() }));
        let failed = r#"{"type":"conversation.item.input_audio_transcription.failed","item_id":"x","error":{"code":"audio_unintelligible","message":"The audio could not be transcribed."}}"#;
        assert!(matches!(transcript.take(&event(failed)), Event::Refused(Failure { kind: FailureKind::Service, .. })));
        transcript.take(&event(r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"y","transcript":"Hello."}"#));
        assert_eq!(transcript.take(&event(failed)), Event::Words);
    }
}
