//! OpenAI's `/audio/transcriptions`, the format the most services, relays and servers of one's own
//! speak (OpenAI, Groq, SiliconFlow, Mistral, whisper servers, …): the sound is kept while the user
//! speaks and goes as one WAV file once they are done, and the words come back whole. No words
//! show while the user speaks. A voice message goes in its own file, as Telegram has it.

use std::sync::mpsc::Receiver;
use std::time::Duration;

use super::audio;
use super::dictation::{Input, Options, Report, Sound, Stopped, iso_639_1};
use super::service::{self, Failure};

/// How long the upload and the transcription may take: minutes of speech go up and come back.
const TIMEOUT: Duration = Duration::from_secs(180);
/// How long a wait for the microphone's next sound lasts before the loop looks again.
const WAIT: Duration = Duration::from_millis(50);

/// The dictation: the sound kept until the user is done, then sent.
pub fn listen(options: &Options, inputs: &Receiver<Input>, report: &dyn Fn(Report)) -> Result<String, Stopped> {
    // Nothing to connect to until the end: the microphone's sound is all that is needed now.
    report(Report::Ready);
    let mut sound = Sound::new(inputs, options.speech.format.rate());
    let mut kept: Vec<i16> = Vec::new();
    while sound.finished.is_none() {
        sound.wait(WAIT)?;
        kept.append(&mut sound.waiting);
    }
    kept.append(&mut sound.waiting);
    // Less than 100 ms is nothing said.
    if kept.len() < sound.rate() as usize / 10 {
        return Ok(String::new());
    }
    transcribe_file(options, "speech.wav", "audio/wav", audio::wav(&kept, sound.rate())).map_err(Stopped::Failed)
}

/// A file's words: `file` (named `name`, of `mime`) sent with the model, the main language and a
/// line in its script for the services that take one.
pub fn transcribe_file(options: &Options, name: &str, mime: &str, file: Vec<u8>) -> Result<String, Failure> {
    let boundary = format!("finchgram{}", service::random_id().replace('-', ""));
    let mut fields = vec![("model", options.speech.model.clone())];
    if let Some(language) = iso_639_1(&options.language) {
        fields.push(("language", language.to_string()));
    }
    if let Some(prompt) = prompt(&options.language, &options.keywords) {
        fields.push(("prompt", prompt));
    }
    let body = multipart(&boundary, &fields, name, mime, &file);
    let content_type = format!("multipart/form-data; boundary={boundary}");
    match service::post(&options.speech.openai_transcriptions(), &service::bearer(options.speech_key.as_deref()), &content_type, body, TIMEOUT) {
        Ok((200..=299, body)) => {
            let answer: serde_json::Value = serde_json::from_str(&body).map_err(|_| Failure::service("the answer is not the words"))?;
            answer["text"].as_str().map(str::to_string).ok_or_else(|| Failure::service("the answer has no words"))
        }
        Ok((status, body)) => Err(Failure::of_status(status, &body)),
        Err(err) => Err(Failure::network(err)),
    }
}

/// For Chinese, a line in the script wanted, with English words left in it: the Whisper family of
/// models writes as its prompt is written, and the newer ones take it as context. For the other
/// languages none (the language is enough, and a line in another language would pull the words
/// toward it), but for the words to know, which the models take from the prompt.
fn prompt(language: &str, keywords: &[String]) -> Option<String> {
    let words = keywords.join(", ");
    match (language, keywords.is_empty()) {
        ("zh-cn", true) => Some("以下是普通话的句子，夹杂的英文照原样写，比如 meeting、OK。".into()),
        ("zh-cn", false) => Some(format!("以下是普通话的句子，夹杂的英文照原样写，比如 meeting、OK。常说的词：{}。", keywords.join("、"))),
        ("zh-tw", true) => Some("以下是國語的句子，夾雜的英文照原樣寫，例如 meeting、OK。".into()),
        ("zh-tw", false) => Some(format!("以下是國語的句子，夾雜的英文照原樣寫，例如 meeting、OK。常說的詞：{}。", keywords.join("、"))),
        (_, true) => None,
        (_, false) => Some(format!("{words}.")),
    }
}

/// A multipart form: the plain fields, then the file.
fn multipart(boundary: &str, fields: &[(&str, String)], name: &str, mime: &str, file: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(file.len() + 1024);
    for (field, value) in fields {
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{field}\"\r\n\r\n{value}\r\n").as_bytes());
    }
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: {mime}\r\n\r\n").as_bytes());
    body.extend_from_slice(file);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_form_holds_the_fields_then_the_file() {
        let body = multipart("b0und", &[("model", "gpt-transcribe".into()), ("language", "zh".into())], "speech.wav", "audio/wav", b"RIFF..");
        let text = String::from_utf8_lossy(&body);
        assert!(text.starts_with("--b0und\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\ngpt-transcribe\r\n--b0und\r\n"));
        assert!(text.contains("name=\"language\"\r\n\r\nzh\r\n"));
        assert!(text.contains("name=\"file\"; filename=\"speech.wav\"\r\nContent-Type: audio/wav\r\n\r\nRIFF..\r\n--b0und--\r\n"));
    }

    #[test]
    fn chinese_gets_a_line_in_its_script_and_the_words_to_know_go_into_the_prompt() {
        assert!(prompt("zh-cn", &[]).is_some_and(|line| line.contains("普通话")));
        assert!(prompt("zh-tw", &[]).is_some_and(|line| line.contains("國語")));
        assert_eq!(prompt("ja", &[]), None);
        assert_eq!(prompt("auto", &[]), None);
        let words = ["陈思远".to_string(), "FinchGram".to_string()];
        assert!(prompt("zh-cn", &words).is_some_and(|line| line.ends_with("常说的词：陈思远、FinchGram。")));
        assert_eq!(prompt("en", &words), Some("陈思远, FinchGram.".to_string()));
    }
}
