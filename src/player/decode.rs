//! A sound file's samples, for a speech service (src/speech/messages.rs: a voice message
//! transcribed by the user's own service): a libmpv of its own decodes the file and writes it with
//! its PCM writer (ao=pcm, which mpv runs as fast as it decodes, not in real time) as one channel
//! of 16-bit samples at the rate asked for, into a WAV file that is read back and deleted at once.
//! A sound file of the kinds a voice message comes in will do (FORMATS): Opus in OGG from
//! Telegram's apps, M4A, MP3.
//!
//! The file is someone else's, and what is decoded goes to the user's AI vendor: mpv opens that
//! one file and nothing the file names (LOCAL_ONLY, mod.rs), and only as one of FORMATS. A file
//! that is a list of other files inside (an edit list, a playlist, FFmpeg's own lists), whatever
//! its name says, would otherwise have a sound file of the Mac, or of an address, decoded in its
//! place and sent.

use std::ffi::{CString, c_char};
use std::path::Path;
use std::ptr;
use std::time::{Duration, Instant};

use super::mpv;

/// How long the decoding may take: minutes of sound decode in a second or two.
const PATIENCE: Duration = Duration::from_secs(60);

/// The only kinds of file decoded, by the names of FFmpeg's demuxers: what voice messages are
/// (OGG from Telegram's apps, M4A from FinchGram and Apple's, MP3), and plain sound files of other
/// apps (AAC, WAV, FLAC, WebM or Matroska, AMR, CAF). Each holds its sound itself. Every other
/// demuxer is refused, the ones that read a list of files and open those among them (concat, HLS,
/// DASH), which mpv's own switch (access-references) does not reach in all of them.
const FORMATS: &str = "format_whitelist=[ogg,mov,mp3,aac,wav,flac,matroska,amr,caf]";

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
        // FFmpeg's demuxers only (none of mpv's own, which are the lists), and of them FORMATS
        // only: said after LOCAL_ONLY, whose word on the demuxer's protocols it repeats.
        let formats = format!("{},{FORMATS}", super::LOCAL_PROTOCOLS);
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
            ("demuxer", "lavf"),
            ("demuxer-lavf-o", formats.as_str()),
        ];
        for (name, value) in super::LOCAL_ONLY.into_iter().chain(options) {
            let (Ok(name), Ok(value)) = (CString::new(name), CString::new(value)) else { continue };
            // An option mpv does not take is one of the walls missing: nothing is decoded.
            if mpv::mpv_set_option_string(handle, name.as_ptr(), value.as_ptr()) < 0 {
                mpv::mpv_terminate_destroy(handle);
                return Err(format!("mpv refused the option {}", name.to_string_lossy()));
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

    fn scratch(name: &str) -> std::path::PathBuf {
        let folder = std::env::temp_dir().join(format!("finchgram-decode-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("a folder");
        folder
    }

    /// A WAV of a tone at 48 kHz, `seconds` long.
    fn tone(seconds: usize) -> Vec<u8> {
        let wave: Vec<i16> = (0..48_000 * seconds).map(|i| ((i as f64 * 440.0 * 2.0 * std::f64::consts::PI / 48_000.0).sin() * 8000.0) as i16).collect();
        crate::speech::wav(&wave, 48_000)
    }

    /// A page of an OGG file: its packets (each under 255 bytes), with the checksum OGG wants.
    fn ogg_page(kind: u8, granule: u64, sequence: u32, packets: &[&[u8]]) -> Vec<u8> {
        let mut page = b"OggS".to_vec();
        page.extend([0, kind]);
        page.extend(granule.to_le_bytes());
        page.extend(0x4649_4E43u32.to_le_bytes());
        page.extend(sequence.to_le_bytes());
        page.extend([0u8; 4]);
        page.push(packets.len() as u8);
        page.extend(packets.iter().map(|packet| packet.len() as u8));
        for packet in packets {
            page.extend_from_slice(packet);
        }
        let mut checksum: u32 = 0;
        for byte in &page {
            checksum ^= u32::from(*byte) << 24;
            for _ in 0..8 {
                checksum = if checksum & 0x8000_0000 != 0 { (checksum << 1) ^ 0x04C1_1DB7 } else { checksum << 1 };
            }
        }
        page[22..26].copy_from_slice(&checksum.to_le_bytes());
        page
    }

    /// A voice message as Telegram's apps send them, Opus in OGG, one channel: a second of
    /// silence, fifty frames of 20 ms with nothing in them.
    fn voice_message() -> Vec<u8> {
        let mut head = b"OpusHead".to_vec();
        head.extend([1, 1]);
        head.extend(0u16.to_le_bytes());
        head.extend(48_000u32.to_le_bytes());
        head.extend(0i16.to_le_bytes());
        head.push(0);
        let mut tags = b"OpusTags".to_vec();
        tags.extend(9u32.to_le_bytes());
        tags.extend(b"FinchGram");
        tags.extend(0u32.to_le_bytes());
        let frames = [&[0xF8u8][..]; 50];
        [ogg_page(2, 0, 0, &[&head]), ogg_page(0, 0, 1, &[&tags]), ogg_page(4, 50 * 960, 2, &frames)].concat()
    }

    /// Needs libmpv (vendor/mpv/bin, scripts/fetch-mpv.sh): a WAV of a tone at 48 kHz comes back at
    /// 16 kHz, as long, faster than it plays; and a voice message of Telegram's apps is decoded.
    #[test]
    #[ignore]
    fn a_sound_file_comes_back_at_the_rate_asked_for_faster_than_it_plays() {
        let scratch = scratch("tone");
        let source = scratch.join("tone.wav");
        std::fs::write(&source, tone(3)).expect("written");
        let started = Instant::now();
        let heard = super::samples(&source, 16_000, &scratch).expect("decoded");
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
        assert!((47_000..=49_000).contains(&heard.len()), "{}", heard.len());
        // Opus in OGG, under the name TDLib gives a voice message's file.
        let voice = scratch.join("5361731456214312345.oga");
        std::fs::write(&voice, voice_message()).expect("written");
        let heard = super::samples(&voice, 16_000, &scratch).expect("the voice message decoded");
        assert!((15_000..=17_000).contains(&heard.len()), "{}", heard.len());
        // A real one, when one is given: FINCHGRAM_TEST_VOICE.
        if let Ok(voice) = std::env::var("FINCHGRAM_TEST_VOICE") {
            let heard = super::samples(std::path::Path::new(&voice), 16_000, &scratch).expect("the voice message decoded");
            println!("{voice}: {} samples at 16 kHz, {:.2} s", heard.len(), heard.len() as f64 / 16_000.0);
            assert!(heard.len() > 16_000);
        }
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// Needs libmpv. A file that is a list of other files inside, under a voice message's name, is
    /// not decoded: neither a sound file of the Mac it names, nor one at an address, which is not
    /// even asked for. (With mpv's own settings every one of these but the playlists came back as
    /// the other file's sound: the security review of 2026-11-13.)
    #[test]
    #[ignore]
    fn a_file_that_names_other_files_is_not_followed() {
        use std::io::Read as _;
        let scratch = scratch("lists");
        let (voices, elsewhere) = (scratch.join("voice"), scratch.join("elsewhere"));
        std::fs::create_dir_all(&voices).expect("a folder");
        std::fs::create_dir_all(&elsewhere).expect("a folder");
        let secret = elsewhere.join("secret.wav");
        std::fs::write(&secret, tone(1)).expect("written");
        std::fs::write(voices.join("beside.wav"), tone(1)).expect("written");
        // An address on this Mac that notes whether anything connected.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
        let address = format!("http://{}/secret.wav", listener.local_addr().expect("the address"));
        listener.set_nonblocking(true).expect("non-blocking");
        let secret = secret.display();
        let lists = [
            ("an edit list", format!("# mpv EDL v0\n{secret}\n")),
            ("an edit list of an address", format!("# mpv EDL v0\n{address}\n")),
            ("a playlist", format!("#EXTM3U\n{secret}\n")),
            ("a playlist of a place beside", "#EXTM3U\n../elsewhere/secret.wav\n".to_string()),
            ("a playlist of an address", format!("#EXTM3U\n{address}\n")),
            ("FFmpeg's list", "ffconcat version 1.0\nfile 'beside.wav'\n".to_string()),
            ("a stream's playlist", format!("#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXTINF:1.0,\nfile://{secret}\n#EXT-X-ENDLIST\n")),
        ];
        for (what, list) in lists {
            let voice = voices.join("5361731456214312345.oga");
            std::fs::write(&voice, list).expect("written");
            let heard = super::samples(&voice, 16_000, &scratch);
            assert!(heard.is_err(), "{what} was followed: {} samples", heard.map_or(0, |heard| heard.len()));
        }
        let mut asked = [0u8; 1];
        let connected = listener.accept().is_ok_and(|(mut connection, _)| connection.read(&mut asked).is_ok());
        assert!(!connected, "an address in a file was asked for");
        // The files themselves are sound files still.
        assert!(super::samples(&voices.join("beside.wav"), 16_000, &scratch).is_ok());
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn the_decoder_keeps_the_local_protocols_when_it_names_its_formats() {
        // The decoder says demuxer-lavf-o again, with its formats: the protocols must be in it.
        assert!(super::super::LOCAL_ONLY.contains(&("demuxer-lavf-o", super::super::LOCAL_PROTOCOLS)));
        assert!(super::super::LOCAL_ONLY.contains(&("access-references", "no")));
        assert!(FORMATS.starts_with("format_whitelist=[") && FORMATS.contains("ogg") && FORMATS.contains("mov") && FORMATS.contains("mp3"));
    }
}
