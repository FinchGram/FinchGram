//! One dictation, from the first sound to the words for the composer, on a thread of its own: the
//! speech service of the profile in use hears it (src/speech/realtime.rs, doubao.rs, upload.rs),
//! then its text model tidies the words (tidy.rs). The same runs for a voice message the user asks
//! to transcribe with their own service, its sound decoded from the file instead of the
//! microphone's.

use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Instant;

use super::audio::Resampler;
use super::service::{Failure, Speech, SpeechFormat, Tidy};
use super::{doubao, realtime, tidy, upload};

/// What the dictation's thread is told: sound, the user's word that they are done, or that the
/// words are not wanted.
pub enum Input {
    Audio { samples: Vec<f32>, rate: f64 },
    Finish,
    Cancel,
}

/// What it tells the UI thread.
pub enum Report {
    /// The service has the session's settings: the sound goes out.
    Ready,
    /// The words so far.
    Words(String),
    /// The last words are in; the text model tidies them.
    Tidying,
    /// The words (tidied, or as heard when tidying failed or is off).
    Done(String),
    Failed(Failure),
}

/// A dictation's settings, read when it starts.
pub struct Options {
    pub speech: Speech,
    /// None for a server of the user's own that asks for no key.
    pub speech_key: Option<String>,
    /// The main language: "auto", or the code of one of the UI's languages ("zh-cn").
    pub language: String,
    /// Names and terms to listen for (the profile's "words to know").
    pub keywords: Vec<String>,
    /// OpenAI's live model's delay; empty for FinchGram's own.
    pub delay: String,
    /// The text model and its key; none for the words as heard.
    pub tidy: Option<(Tidy, Option<String>)>,
    /// The sound is a file's, all of it there at once (a voice message): it goes out as fast as
    /// the service takes it, not as it is spoken.
    pub recorded: bool,
}

/// Why a session ended without words.
pub enum Stopped {
    Cancelled,
    Failed(Failure),
}

impl From<Failure> for Stopped {
    fn from(failure: Failure) -> Stopped {
        Stopped::Failed(failure)
    }
}

/// One dictation; `report` tells the UI thread how it goes. Returns when it is done, failed, or
/// was told it is not wanted.
pub fn dictate(options: &Options, inputs: &Receiver<Input>, report: &dyn Fn(Report)) {
    let heard = match options.speech.format {
        SpeechFormat::OpenAiRealtime | SpeechFormat::QwenRealtime => realtime::listen(options, inputs, report),
        SpeechFormat::OpenAiTranscriptions => upload::listen(options, inputs, report),
        SpeechFormat::Doubao => doubao::listen(options, inputs, report),
    };
    match heard {
        Ok(heard) => {
            let heard = heard.trim().to_string();
            let words = match &options.tidy {
                Some((service, key)) if !heard.is_empty() => {
                    report(Report::Tidying);
                    tidy::tidy(service, key.as_deref(), &options.language, &heard).unwrap_or_else(|err| {
                        eprintln!("voice input: the words stay as heard: {err}");
                        heard.clone()
                    })
                }
                _ => heard,
            };
            report(Report::Done(words));
        }
        Err(Stopped::Cancelled) => {}
        Err(Stopped::Failed(failure)) => report(Report::Failed(failure)),
    }
}

/// The sound as a session takes it: what the microphone (or a file) gave, at the session's rate,
/// 16-bit, waiting to go out; and whether the user is done.
pub struct Sound<'a> {
    inputs: &'a Receiver<Input>,
    rate: u32,
    resampler: Option<Resampler>,
    pub waiting: Vec<i16>,
    /// How many samples came in all.
    pub total: usize,
    pub finished: Option<Instant>,
}

impl<'a> Sound<'a> {
    pub fn new(inputs: &'a Receiver<Input>, rate: u32) -> Sound<'a> {
        Sound { inputs, rate, resampler: None, waiting: Vec::new(), total: 0, finished: None }
    }

    /// Everything that came since the last look; Err when the words are not wanted.
    pub fn take(&mut self) -> Result<(), Stopped> {
        loop {
            match self.inputs.try_recv() {
                Ok(Input::Audio { samples, rate }) => {
                    if self.finished.is_none() {
                        let before = self.waiting.len();
                        let to = self.rate;
                        self.resampler.get_or_insert_with(|| Resampler::new(rate, to)).push(&samples, &mut self.waiting);
                        self.total += self.waiting.len() - before;
                    }
                }
                Ok(Input::Finish) => self.end(),
                Ok(Input::Cancel) | Err(TryRecvError::Disconnected) => return Err(Stopped::Cancelled),
                Err(TryRecvError::Empty) => return Ok(()),
            }
        }
    }

    /// Waits for the next input when nothing waits to go out: a file's sound, sent all at once,
    /// needs no polling, and the microphone's comes every few milliseconds anyway.
    pub fn wait(&mut self, at_most: std::time::Duration) -> Result<(), Stopped> {
        match self.inputs.recv_timeout(at_most) {
            Ok(Input::Audio { samples, rate }) => {
                if self.finished.is_none() {
                    let before = self.waiting.len();
                    let to = self.rate;
                    self.resampler.get_or_insert_with(|| Resampler::new(rate, to)).push(&samples, &mut self.waiting);
                    self.total += self.waiting.len() - before;
                }
                self.take()
            }
            Ok(Input::Finish) => {
                self.end();
                self.take()
            }
            Ok(Input::Cancel) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(Stopped::Cancelled),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Ok(()),
        }
    }

    /// The user is done: the last samples the resampler held back come out, once.
    fn end(&mut self) {
        if self.finished.is_none() {
            self.finished = Some(Instant::now());
            if let Some(resampler) = self.resampler.as_mut() {
                let before = self.waiting.len();
                resampler.finish(&mut self.waiting);
                self.total += self.waiting.len() - before;
            }
        }
    }

    /// The next piece of `size` samples to send, or the last, shorter one once the user is done.
    pub fn piece(&mut self, size: usize) -> Option<Vec<i16>> {
        if self.waiting.len() >= size || (self.finished.is_some() && !self.waiting.is_empty()) {
            Some(self.waiting.drain(..self.waiting.len().min(size)).collect())
        } else {
            None
        }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }
}

/// The words to know as typed: separated by commas (any script's) or lines, each on one line, of
/// plain characters (no angle brackets, which OpenAI refuses), at most 50 of them.
pub fn keywords(text: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    for word in text.split([',', '，', '、', ';', '；', '\n']) {
        let word: String = word.trim().chars().filter(|character| !matches!(character, '<' | '>') && !character.is_control()).take(60).collect();
        if !word.is_empty() && !words.contains(&word) && words.len() < 50 {
            words.push(word);
        }
    }
    words
}

/// The main language as a service's hint: ISO 639-1 ("zh", "en"), the script left aside.
pub fn iso_639_1(language: &str) -> Option<&str> {
    match language {
        "auto" | "" => None,
        "zh-cn" | "zh-tw" => Some("zh"),
        other => Some(other),
    }
}

/// The main language in English, for a model's instructions.
pub fn english_name(language: &str) -> Option<&'static str> {
    Some(match language {
        "en" => "English",
        "zh-cn" => "Chinese (Mandarin, written in Simplified characters)",
        "zh-tw" => "Chinese (Mandarin, written in Traditional characters as in Taiwan)",
        "ja" => "Japanese",
        "ko" => "Korean",
        "fr" => "French",
        "de" => "German",
        "es" => "Spanish",
        "pt" => "Portuguese",
        "ru" => "Russian",
        "ar" => "Arabic",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn sound_comes_at_the_sessions_rate_and_goes_in_pieces_then_the_rest() {
        let (inputs, received) = mpsc::channel();
        let mut sound = Sound::new(&received, 16_000);
        inputs.send(Input::Audio { samples: vec![0.25; 4800], rate: 48_000.0 }).expect("sent");
        sound.take().map_err(|_| "cancelled").expect("taken");
        // A few samples wait for sound after them.
        assert!((1570..=1600).contains(&sound.waiting.len()), "{}", sound.waiting.len());
        assert!(sound.piece(3200).is_none());
        assert_eq!(sound.piece(800).map(|piece| piece.len()), Some(800));
        inputs.send(Input::Finish).expect("sent");
        // Sound after the end is not taken.
        inputs.send(Input::Audio { samples: vec![0.25; 4800], rate: 48_000.0 }).expect("sent");
        sound.take().map_err(|_| "cancelled").expect("taken");
        assert!(sound.finished.is_some());
        // The rest, with what the resampler held back: the 100 ms in all, give or take a sample.
        let rest = sound.piece(3200).expect("the rest");
        assert!((798..=801).contains(&rest.len()), "{}", rest.len());
        assert!(sound.piece(3200).is_none());
        inputs.send(Input::Cancel).expect("sent");
        assert!(matches!(sound.take(), Err(Stopped::Cancelled)));
    }

    #[test]
    fn words_to_know_are_split_by_any_scripts_commas_and_kept_plain() {
        assert_eq!(keywords("FinchGram, 陈思远，Seed-ASR、 Kubernetes\nPikPak; <b>x</b> ,, FinchGram"), vec!["FinchGram", "陈思远", "Seed-ASR", "Kubernetes", "PikPak", "bx/b"]);
        assert!(keywords("  ").is_empty());
        assert_eq!(keywords(&"word,".repeat(80)).len(), 1);
    }

    #[test]
    fn the_main_language_becomes_each_services_hint() {
        assert_eq!(iso_639_1("zh-tw"), Some("zh"));
        assert_eq!(iso_639_1("ja"), Some("ja"));
        assert_eq!(iso_639_1("auto"), None);
        assert!(english_name("zh-tw").is_some_and(|name| name.contains("Traditional")));
        assert_eq!(english_name("auto"), None);
    }
}
