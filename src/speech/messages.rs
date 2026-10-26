//! A voice message's words by the user's own speech service: the design's Transcribe (the fifth
//! round, B1) when Telegram's own transcription cannot be had (src/telegram/voice.rs) and the
//! user allows it (`[voice_input] transcribe_messages`, off until they turn it on in the panel).
//! It hands Telegram's content, someone's voice, to the service the user chose, for the one
//! message they clicked (docs/voice-input.md); the words are kept in memory for the session
//! (the store's own_transcripts) and written nowhere.
//!
//! The file goes as it is to the upload format; for the realtime formats libmpv decodes it to the
//! service's rate first (src/player/decode.rs), and the sound goes out as fast as the service
//! takes it. No tidying: the words are someone else's, as they said them.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;

use super::dictation::{self, Input, Options, Report};
use super::service::SpeechFormat;
use super::upload;

type Done = Box<dyn FnOnce(Result<String, String>)>;

thread_local! {
    /// What waits for each transcription, by its number.
    static WAITING: RefCell<(u64, HashMap<u64, Done>)> = RefCell::new((1, HashMap::new()));
}

/// The words of the voice message in the file at `path` (of `mime`), through `options`'
/// service; `done` gets them, or why not, on the UI thread.
pub fn transcribe(options: Options, path: PathBuf, mime: String, done: impl FnOnce(Result<String, String>) + 'static) {
    let number = WAITING.with(|waiting| {
        let mut waiting = waiting.borrow_mut();
        let number = waiting.0;
        waiting.0 += 1;
        waiting.1.insert(number, Box::new(done));
        number
    });
    let spawned = std::thread::Builder::new().name("voice-message".into()).spawn(move || {
        let words = words_of(&options, &path, &mime);
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(done) = WAITING.with(|waiting| waiting.borrow_mut().1.remove(&number)) {
                done(words);
            }
        });
    });
    if let Err(err) = spawned
        && let Some(done) = WAITING.with(|waiting| waiting.borrow_mut().1.remove(&number))
    {
        done(Err(err.to_string()));
    }
}

fn words_of(options: &Options, path: &std::path::Path, mime: &str) -> Result<String, String> {
    if options.speech.format == SpeechFormat::OpenAiTranscriptions {
        let file = std::fs::read(path).map_err(|err| err.to_string())?;
        return upload::transcribe_file(options, &file_name(path, mime), mime_or_default(mime), file).map_err(|failure| failure.detail);
    }
    let rate = options.speech.format.rate();
    let scratch = dirs::cache_dir().ok_or("no caches folder")?.join("FinchGram").join("decoding");
    let samples = crate::player::decode::samples(path, rate, &scratch)?;
    let (inputs, received) = mpsc::channel();
    let sound: Vec<f32> = samples.iter().map(|sample| f32::from(*sample) / 32768.0).collect();
    let _ = inputs.send(Input::Audio { samples: sound, rate: f64::from(rate) });
    let _ = inputs.send(Input::Finish);
    let outcome = RefCell::new(None);
    dictation::dictate(options, &received, &|report| match report {
        Report::Done(words) => *outcome.borrow_mut() = Some(Ok(words)),
        Report::Failed(failure) => *outcome.borrow_mut() = Some(Err(failure.detail)),
        _ => {}
    });
    // The sender lives until the session is over: a closed channel would be taken as a cancel.
    drop(inputs);
    outcome.into_inner().unwrap_or_else(|| Err("the service said nothing".into()))
}

/// The file's name as the upload carries it: what its kind is, for the services that go by it.
fn file_name(path: &std::path::Path, mime: &str) -> String {
    let extension = match mime {
        "audio/ogg" | "audio/opus" => "ogg",
        "audio/mp4" | "audio/m4a" | "audio/x-m4a" | "audio/aac" => "m4a",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/wav" | "audio/x-wav" => "wav",
        _ => path.extension().and_then(|extension| extension.to_str()).unwrap_or("ogg"),
    };
    format!("voice.{extension}")
}

fn mime_or_default(mime: &str) -> &str {
    if mime.is_empty() { "audio/ogg" } else { mime }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_upload_names_the_files_kind() {
        assert_eq!(file_name(std::path::Path::new("/x/file_12"), "audio/ogg"), "voice.ogg");
        assert_eq!(file_name(std::path::Path::new("/x/file_12.m4a"), ""), "voice.m4a");
        assert_eq!(file_name(std::path::Path::new("/x/file_12"), "audio/mpeg"), "voice.mp3");
        assert_eq!(mime_or_default(""), "audio/ogg");
    }
}
