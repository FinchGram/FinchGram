//! A sound file's samples, for a speech service (src/speech/messages.rs: a voice message
//! transcribed by the user's own service): a libmpv of its own decodes the file and writes it with
//! its PCM writer (ao=pcm, which mpv runs as fast as it decodes, not in real time) as one channel
//! of 16-bit samples at the rate asked for, into a WAV file that is read back and deleted at once.
//! Any file mpv plays will do: Opus in OGG from Telegram's apps, M4A, MP3.

use std::ffi::{CString, c_char};
use std::path::Path;
use std::ptr;
use std::time::{Duration, Instant};

use super::mpv;

/// How long the decoding may take: minutes of sound decode in a second or two.
const PATIENCE: Duration = Duration::from_secs(60);

/// The sound of `path` as 16-bit samples of one channel at `rate`, decoded on this thread (call it
/// from one of the speech's own). `scratch` is the folder the WAV file goes into for a moment.
pub fn samples(path: &Path, rate: u32, scratch: &Path) -> Result<Vec<i16>, String> {
    std::fs::create_dir_all(scratch).map_err(|err| err.to_string())?;
    let out = scratch.join(format!("decoded-{}-{}.wav", std::process::id(), next_number()));
    let result = decode(path, rate, &out).and_then(|()| {
        let file = std::fs::read(&out).map_err(|err| err.to_string())?;
        let (samples, got) = crate::speech::wav_samples(&file).ok_or("mpv wrote no sound")?;
        if got != rate {
            return Err(format!("mpv wrote {got} Hz, not {rate}"));
        }
        Ok(samples)
    });
    let _ = std::fs::remove_file(&out);
    result
}

/// A number to keep two decodings at once from sharing a file.
fn next_number() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn decode(path: &Path, rate: u32, out: &Path) -> Result<(), String> {
    // SAFETY: a libmpv handle of our own, used on this thread only and destroyed before leaving;
    // the strings outlive the calls that take them.
    unsafe {
        let handle = mpv::mpv_create();
        if handle.is_null() {
            return Err("mpv_create failed".into());
        }
        let rate = rate.to_string();
        let out = out.to_string_lossy().to_string();
        let options = [
            ("config", "no"),
            ("terminal", "no"),
            ("msg-level", "all=no"),
            ("vid", "no"),
            ("vo", "null"),
            ("audio-display", "no"),
            ("ao", "pcm"),
            ("ao-pcm-file", out.as_str()),
            ("ao-pcm-waveheader", "yes"),
            ("audio-samplerate", rate.as_str()),
            ("audio-channels", "mono"),
            ("audio-format", "s16"),
        ];
        for (name, value) in options {
            let (Ok(name), Ok(value)) = (CString::new(name), CString::new(value)) else { continue };
            if mpv::mpv_set_option_string(handle, name.as_ptr(), value.as_ptr()) < 0 {
                eprintln!("decode: mpv refused the option {}", name.to_string_lossy());
            }
        }
        if mpv::mpv_initialize(handle) < 0 {
            mpv::mpv_terminate_destroy(handle);
            return Err("mpv did not start".into());
        }
        let (Ok(loadfile), Ok(file)) = (CString::new("loadfile"), CString::new(path.to_string_lossy().as_bytes())) else {
            mpv::mpv_terminate_destroy(handle);
            return Err("a path mpv cannot take".into());
        };
        let mut args: [*const c_char; 3] = [loadfile.as_ptr(), file.as_ptr(), ptr::null()];
        if mpv::mpv_command(handle, args.as_mut_ptr()) < 0 {
            mpv::mpv_terminate_destroy(handle);
            return Err("mpv did not take the file".into());
        }
        let started = Instant::now();
        let mut result = Err("mpv took too long".to_string());
        while started.elapsed() < PATIENCE {
            let event = &*mpv::mpv_wait_event(handle, 0.25);
            match event.event_id {
                mpv::MPV_EVENT_END_FILE => {
                    let ended = &*(event.data as *const mpv::mpv_event_end_file);
                    result = if ended.reason == mpv::MPV_END_FILE_REASON_EOF { Ok(()) } else { Err(format!("mpv could not decode it ({})", super::error_string(ended.error))) };
                    break;
                }
                mpv::MPV_EVENT_SHUTDOWN => break,
                _ => {}
            }
        }
        // The PCM writer writes the WAV header's sizes when it closes.
        mpv::mpv_terminate_destroy(handle);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs libmpv (vendor/mpv/bin, scripts/fetch-mpv.sh): a WAV of a tone at 48 kHz comes back at
    /// 16 kHz, as long, faster than it plays.
    #[test]
    #[ignore]
    fn a_sound_file_comes_back_at_the_rate_asked_for_faster_than_it_plays() {
        let scratch = std::env::temp_dir().join(format!("finchgram-decode-test-{}", std::process::id()));
        std::fs::create_dir_all(&scratch).expect("a folder");
        let wave: Vec<i16> = (0..48_000 * 3).map(|i| ((i as f64 * 440.0 * 2.0 * std::f64::consts::PI / 48_000.0).sin() * 8000.0) as i16).collect();
        let source = scratch.join("tone.wav");
        std::fs::write(&source, crate::speech::wav(&wave, 48_000)).expect("written");
        let started = Instant::now();
        let tone = super::samples(&source, 16_000, &scratch).expect("decoded");
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
        assert!((47_000..=49_000).contains(&tone.len()), "{}", tone.len());
        // A voice message of Telegram's apps (Opus in OGG), when one is given: FINCHGRAM_TEST_VOICE.
        if let Ok(voice) = std::env::var("FINCHGRAM_TEST_VOICE") {
            let heard = super::samples(std::path::Path::new(&voice), 16_000, &scratch).expect("the voice message decoded");
            println!("{voice}: {} samples at 16 kHz, {:.2} s", heard.len(), heard.len() as f64 / 16_000.0);
            assert!(heard.len() > 16_000);
        }
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
