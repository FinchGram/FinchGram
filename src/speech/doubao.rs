//! ByteDance's Doubao streaming recognition (Seed-ASR) on Volcengine, 豆包流式语音识别, in its own
//! binary protocol over a WebSocket (its document of 2026-08-06).
//!
//! Every frame is a 4-byte header (protocol version and header size; message type and its flags;
//! serialization and compression), then for the server's answers a sequence number, then the
//! payload's size (big-endian) and the payload. The client sends its request first (JSON: the
//! sound's format, the options), then the sound in pieces of 200 ms, the last one flagged; the
//! server answers each with the whole text so far, and the answer to the last piece is the final
//! one.
//!
//! Chinese, English and any language go to the two-pass endpoint (`bigmodel_async` with
//! `enable_nonstream`): words while the user speaks, each sentence heard again for the final
//! words. The other languages Doubao names one by one (Japanese, Korean, …) go to `bigmodel_nostream`
//! with their code: the words come at the end. A voice message's sound, all there at once, goes to
//! the latter too. Traditional Chinese is written by Doubao itself (`output_zh_variant`).
//!
//! The key is the new console's API key (`X-Api-Key`); the old console's App ID and Access Token
//! go into the key's field as "appid:token". The model is the resource the dictation is billed
//! under (`X-Api-Resource-Id`): Seed-ASR 2.0 by the hour unless the user names another.

use std::io::Read as _;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tungstenite::Message;

use super::audio;
use super::dictation::{Input, Options, Report, Sound, Stopped};
use super::service::{self, Failure, FailureKind};

const VERSION_AND_HEADER_SIZE: u8 = 0b0001_0001;
const FULL_CLIENT_REQUEST: u8 = 0b0001;
const AUDIO_ONLY: u8 = 0b0010;
const FULL_SERVER_RESPONSE: u8 = 0b1001;
const SERVER_ERROR: u8 = 0b1111;
/// The flags of a frame: no sequence number; the last piece of sound.
const NO_SEQUENCE: u8 = 0b0000;
const LAST: u8 = 0b0010;
/// A sequence number follows the header (the server's answers).
const WITH_SEQUENCE: u8 = 0b0001;
const RAW: u8 = 0b0000;
const JSON: u8 = 0b0001;
const NO_COMPRESSION: u8 = 0b0000;
const GZIP: u8 = 0b0001;

/// Doubao's "empty sound" error: nothing was heard.
const EMPTY_AUDIO: u32 = 45_000_002;
/// The answers' code for success.
const SUCCESS: u32 = 20_000_000;

/// How long the last words may take once the last piece went.
const LAST_WORDS_WAIT: Duration = Duration::from_secs(20);
/// The most an answer may hold, unpacked: the words of a dictation with their timing are some
/// kilobytes. A frame that unpacks to more is not read further, so that a service, or something
/// standing in for it, cannot have the app unpack a few kilobytes into all its memory.
const MOST_UNPACKED: u64 = 4 * 1024 * 1024;
/// A recorded sound's pieces go this far apart: faster than spoken, slow enough for the service,
/// which wants its pieces paced.
const FILE_PACE: Duration = Duration::from_millis(40);

/// The headers of the handshake: the key (or the old console's App ID and Access Token), the
/// resource billed, an id for the connection.
pub fn headers(key: &str, resource: &str) -> Vec<(&'static str, String)> {
    let mut headers = Vec::new();
    match key.split_once(':') {
        Some((app, token)) if !app.is_empty() && app.chars().all(|character| character.is_ascii_digit()) => {
            headers.push(("X-Api-App-Key", app.to_string()));
            headers.push(("X-Api-Access-Key", token.to_string()));
        }
        _ if !key.is_empty() => headers.push(("X-Api-Key", key.to_string())),
        _ => {}
    }
    headers.push(("X-Api-Resource-Id", resource.to_string()));
    headers.push(("X-Api-Connect-Id", service::random_id()));
    headers
}

/// Whether the main language goes to the two-pass endpoint: the Chinese and English model, which
/// also takes Chinese's dialects, and any language.
fn two_pass(language: &str) -> bool {
    matches!(language, "auto" | "zh-cn" | "zh-tw" | "en" | "")
}

/// Doubao's code for a language of the other model; none for the Chinese and English one.
fn language_code(language: &str) -> Option<&'static str> {
    Some(match language {
        "ja" => "ja-JP",
        "ko" => "ko-KR",
        "fr" => "fr-FR",
        "de" => "de-DE",
        "es" => "es-MX",
        "pt" => "pt-BR",
        "ru" => "ru-RU",
        "ar" => "ar-SA",
        _ => return None,
    })
}

/// The request that opens the session, with the words to know as hot words.
fn request(language: &str, two_pass: bool, keywords: &[String]) -> Value {
    let mut audio = json!({ "format": "pcm", "codec": "raw", "rate": 16000, "bits": 16, "channel": 1 });
    if !two_pass && let Some(code) = language_code(language) {
        audio["language"] = json!(code);
    }
    let mut request = json!({
        "model_name": "bigmodel",
        "enable_itn": true,
        "enable_punc": true,
        "result_type": "full",
    });
    if two_pass {
        request["enable_nonstream"] = json!(true);
    }
    if language == "zh-tw" {
        request["output_zh_variant"] = json!("tw");
    }
    if !keywords.is_empty() {
        let hot_words: Vec<Value> = keywords.iter().map(|word| json!({ "word": word })).collect();
        request["corpus"] = json!({ "context": json!({ "hotwords": hot_words }).to_string() });
    }
    json!({ "audio": audio, "request": request })
}

/// A frame of the client's.
fn frame(kind: u8, flags: u8, serialization: u8, payload: &[u8]) -> Vec<u8> {
    let mut frame = vec![VERSION_AND_HEADER_SIZE, (kind << 4) | flags, (serialization << 4) | NO_COMPRESSION, 0];
    frame.extend_from_slice(&u32::try_from(payload.len()).unwrap_or(u32::MAX).to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

/// A frame of the server's.
#[derive(Debug, PartialEq)]
enum Answer {
    /// The text so far; `last` for the answer to the last piece.
    Words { text: String, last: bool },
    Error { code: u32, message: String },
    Other,
}

fn parse(frame: &[u8]) -> Result<Answer, String> {
    if frame.len() < 4 {
        return Err("a frame too short".into());
    }
    let header = usize::from(frame[0] & 0x0f) * 4;
    let (kind, flags) = (frame[1] >> 4, frame[1] & 0x0f);
    let compression = frame[2] & 0x0f;
    let number = |at: usize| -> Result<u32, String> {
        frame.get(at..at + 4).and_then(|bytes| bytes.try_into().ok()).map(u32::from_be_bytes).ok_or_else(|| "a frame cut short".to_string())
    };
    let payload = |at: usize, size: usize| -> Result<Vec<u8>, String> {
        let bytes = frame.get(at..at + size).ok_or("a frame cut short")?;
        let too_much = || format!("a frame holds more than {MOST_UNPACKED} bytes");
        if compression == GZIP {
            let mut unpacked = Vec::new();
            // One byte past the most, to know that there was more.
            flate2::read::GzDecoder::new(bytes).take(MOST_UNPACKED + 1).read_to_end(&mut unpacked).map_err(|err| err.to_string())?;
            if unpacked.len() as u64 > MOST_UNPACKED {
                return Err(too_much());
            }
            Ok(unpacked)
        } else if bytes.len() as u64 > MOST_UNPACKED {
            Err(too_much())
        } else {
            Ok(bytes.to_vec())
        }
    };
    match kind {
        FULL_SERVER_RESPONSE => {
            let mut at = header;
            if flags & WITH_SEQUENCE != 0 {
                at += 4;
            }
            let size = number(at)? as usize;
            let body: Value = serde_json::from_slice(&payload(at + 4, size)?).map_err(|err| err.to_string())?;
            let result = if body["result"].is_array() { &body["result"][0] } else { &body["result"] };
            Ok(Answer::Words { text: result["text"].as_str().unwrap_or("").to_string(), last: flags & LAST != 0 })
        }
        SERVER_ERROR => {
            let code = number(header)?;
            let size = number(header + 4)? as usize;
            let body = payload(header + 8, size)?;
            let text = String::from_utf8_lossy(&body).to_string();
            let message = service::error_message(&text).unwrap_or(text);
            Ok(Answer::Error { code, message })
        }
        _ => Ok(Answer::Other),
    }
}

/// The session: the request, the sound in pieces, the text as it comes, the final words.
pub fn listen(options: &Options, inputs: &Receiver<Input>, report: &dyn Fn(Report)) -> Result<String, Stopped> {
    let two_pass = two_pass(&options.language) && !options.recorded;
    let headers = headers(options.speech_key.as_deref().unwrap_or(""), &options.speech.model);
    let mut socket = service::connect(&options.speech.doubao(two_pass), &headers)?;
    let opening = request(&options.language, two_pass, &options.keywords).to_string();
    send(&mut socket, frame(FULL_CLIENT_REQUEST, NO_SEQUENCE, JSON, opening.as_bytes()))?;
    // The sound may follow at once: the two-pass endpoint answers only when the text changes, and
    // a request it refuses comes back as an error frame.
    report(Report::Ready);
    let mut sound = Sound::new(inputs, options.speech.format.rate());
    // 200 ms a piece, as Doubao works best with.
    let piece = sound.rate() as usize / 5;
    let mut last_sent: Option<Instant> = None;
    let mut paced = Instant::now();
    let mut text = String::new();
    loop {
        if let Err(stopped) = sound.take() {
            let _ = socket.close(None);
            return Err(stopped);
        }
        if last_sent.is_none() && (!options.recorded || paced.elapsed() >= FILE_PACE) {
            if let Some(samples) = sound.piece(piece) {
                let last = sound.finished.is_some() && sound.waiting.is_empty();
                send(&mut socket, frame(AUDIO_ONLY, if last { LAST } else { NO_SEQUENCE }, RAW, &audio::bytes(&samples)))?;
                paced = Instant::now();
                if last {
                    last_sent = Some(Instant::now());
                }
            } else if sound.finished.is_some() {
                // The user is done with nothing left to send: an empty last piece ends it.
                send(&mut socket, frame(AUDIO_ONLY, LAST, RAW, &[]))?;
                last_sent = Some(Instant::now());
            }
        }
        match socket.read() {
            Ok(Message::Binary(data)) => match parse(&data).map_err(|err| Stopped::Failed(Failure::service(err)))? {
                Answer::Words { text: words, last } => {
                    if words != text {
                        text = words;
                        report(Report::Words(text.clone()));
                    }
                    if last {
                        let _ = socket.close(None);
                        return Ok(text);
                    }
                }
                Answer::Error { code, .. } if code == EMPTY_AUDIO => {
                    let _ = socket.close(None);
                    return Ok(String::new());
                }
                Answer::Error { code, message } if code != SUCCESS => {
                    let _ = socket.close(None);
                    let kind = if (45_000_000..46_000_000).contains(&code) && message.to_lowercase().contains("auth") { FailureKind::RefusedKey } else { FailureKind::Service };
                    return Err(Stopped::Failed(Failure { kind, detail: format!("{message} ({code})") }));
                }
                Answer::Error { .. } | Answer::Other => {}
            },
            Ok(Message::Close(frame)) => {
                if last_sent.is_some() {
                    return Ok(text);
                }
                let reason = frame.map(|frame| frame.reason.to_string()).filter(|reason| !reason.is_empty());
                return Err(Stopped::Failed(Failure::network(reason.unwrap_or_else(|| "Doubao closed the connection".into()))));
            }
            Ok(_) => {}
            Err(err) if service::nothing_yet(&err) => {}
            Err(err) => return Err(Stopped::Failed(Failure::network(err))),
        }
        if last_sent.is_some_and(|at| at.elapsed() > LAST_WORDS_WAIT) {
            let _ = socket.close(None);
            return Ok(text);
        }
    }
}

fn send(socket: &mut service::Socket, frame: Vec<u8>) -> Result<(), Stopped> {
    socket.send(Message::binary(frame)).map_err(|err| Stopped::Failed(Failure::network(err)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_goes_as_the_new_consoles_or_the_old_ones_pair() {
        let new_console = headers("6f1c2e9a-key", "volc.seedasr.sauc.duration");
        assert_eq!(new_console[0], ("X-Api-Key", "6f1c2e9a-key".to_string()));
        assert_eq!(new_console[1], ("X-Api-Resource-Id", "volc.seedasr.sauc.duration".to_string()));
        assert_eq!((new_console[2].0, new_console[2].1.len()), ("X-Api-Connect-Id", 36));
        let old_console = headers("123456789:access-token", "volc.bigasr.sauc.duration");
        assert_eq!(&old_console[..2], &[("X-Api-App-Key", "123456789".to_string()), ("X-Api-Access-Key", "access-token".to_string())]);
        // A colon in a new key is just part of it.
        assert_eq!(headers("ab:cd", "r")[0], ("X-Api-Key", "ab:cd".to_string()));
    }

    #[test]
    fn chinese_and_english_go_two_pass_and_the_others_name_their_language() {
        let opening = request("zh-tw", true, &[]);
        assert_eq!(opening["audio"], json!({ "format": "pcm", "codec": "raw", "rate": 16000, "bits": 16, "channel": 1 }));
        assert_eq!(opening["request"]["enable_nonstream"], true);
        assert_eq!(opening["request"]["output_zh_variant"], "tw");
        assert_eq!(opening["request"]["model_name"], "bigmodel");
        assert!(two_pass("auto") && two_pass("en") && !two_pass("ja"));
        let opening = request("ja", false, &[]);
        assert_eq!(opening["audio"]["language"], "ja-JP");
        assert!(opening["request"].get("enable_nonstream").is_none() && opening["request"].get("output_zh_variant").is_none());
        // Chinese heard from a file: the Chinese model, with no language named.
        assert!(request("zh-cn", false, &[])["audio"].get("language").is_none());
        // The words to know go as hot words, a JSON string as Doubao wants it.
        let opening = request("zh-cn", true, &["陈思远".to_string(), "FinchGram".to_string()]);
        let context: Value = serde_json::from_str(opening["request"]["corpus"]["context"].as_str().expect("a string")).expect("JSON");
        assert_eq!(context, json!({ "hotwords": [{ "word": "陈思远" }, { "word": "FinchGram" }] }));
    }

    #[test]
    fn frames_carry_a_header_a_big_endian_size_and_the_payload() {
        let frame = frame(AUDIO_ONLY, LAST, RAW, &[1, 2, 3]);
        assert_eq!(frame, vec![0x11, 0x22, 0x00, 0x00, 0, 0, 0, 3, 1, 2, 3]);
        let opening = super::frame(FULL_CLIENT_REQUEST, NO_SEQUENCE, JSON, b"{}");
        assert_eq!(&opening[..8], &[0x11, 0x10, 0x10, 0x00, 0, 0, 0, 2]);
    }

    fn answer(flags: u8, payload: &[u8]) -> Vec<u8> {
        let mut frame = vec![0x11, (FULL_SERVER_RESPONSE << 4) | flags, 0x10, 0x00];
        frame.extend_from_slice(&7i32.to_be_bytes());
        frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        frame.extend_from_slice(payload);
        frame
    }

    #[test]
    fn the_servers_answers_give_the_text_and_say_which_is_last() {
        let body = r#"{"audio_info":{"duration":3696},"result":{"text":"这是字节跳动，今日头条母公司。","utterances":[]}}"#;
        assert_eq!(parse(&answer(0b0001, body.as_bytes())), Ok(Answer::Words { text: "这是字节跳动，今日头条母公司。".into(), last: false }));
        assert_eq!(parse(&answer(0b0011, body.as_bytes())), Ok(Answer::Words { text: "这是字节跳动，今日头条母公司。".into(), last: true }));
        // Compressed, as the server may answer.
        let mut packed = Vec::new();
        {
            use std::io::Write as _;
            let mut encoder = flate2::write::GzEncoder::new(&mut packed, flate2::Compression::default());
            encoder.write_all(body.as_bytes()).expect("written");
            encoder.finish().expect("finished");
        }
        let mut compressed = answer(0b0011, &packed);
        compressed[2] = 0x11;
        assert!(matches!(parse(&compressed), Ok(Answer::Words { last: true, .. })));
        // A frame that unpacks to more than any answer could hold is refused, before it is held
        // whole: 16 MB of spaces pack into some kilobytes.
        let mut bomb = Vec::new();
        {
            use std::io::Write as _;
            let mut encoder = flate2::write::GzEncoder::new(&mut bomb, flate2::Compression::default());
            let spaces = vec![b' '; 1024 * 1024];
            for _ in 0..16 {
                encoder.write_all(&spaces).expect("written");
            }
            encoder.finish().expect("finished");
        }
        assert!(bomb.len() < 64 * 1024, "{}", bomb.len());
        let mut compressed = answer(0b0011, &bomb);
        compressed[2] = 0x11;
        assert_eq!(parse(&compressed), Err(format!("a frame holds more than {MOST_UNPACKED} bytes")));
        // An error frame: its code and message.
        let message = r#"{"error":"empty audio"}"#;
        let mut error = vec![0x11, 0xF0, 0x10, 0x00];
        error.extend_from_slice(&EMPTY_AUDIO.to_be_bytes());
        error.extend_from_slice(&(message.len() as u32).to_be_bytes());
        error.extend_from_slice(message.as_bytes());
        assert_eq!(parse(&error), Ok(Answer::Error { code: EMPTY_AUDIO, message: "empty audio".into() }));
        assert!(parse(&[0x11]).is_err());
        assert!(parse(&answer(0b0001, b"{")[..10]).is_err());
    }
}
