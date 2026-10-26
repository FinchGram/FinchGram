//! The microphone on macOS: AVFoundation's question whether FinchGram may listen; for voice input
//! (src/speech/), AVAudioEngine's input node with a tap on it; for voice messages
//! (src/telegram/voice.rs), AVAudioRecorder. The tap's block runs on one of the engine's own
//! threads with the sound in the input's format (floating point, the device's rate, one buffer per
//! channel); it mixes the channels down to one and hands the samples on, and nothing of them stays
//! here. The recorder writes AAC into an M4A file, which Telegram takes as a voice message
//! (TDLib's inputVoiceNote), and says how loud the microphone is.
//!
//! The app needs NSMicrophoneUsageDescription in its Info.plist and, under the hardened runtime,
//! the entitlement com.apple.security.device.audio-input (scripts/bundle.sh); without the
//! entitlement a notarized app hears silence and the system never asks.

use std::ptr::NonNull;
use std::sync::Mutex;

use block2::RcBlock;
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool};
use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};
use objc2_avf_audio::{
    AVAudioEngine, AVAudioPCMBuffer, AVAudioRecorder, AVAudioTime, AVEncoderBitRateKey, AVFormatIDKey, AVNumberOfChannelsKey, AVSampleRateKey,
};
use objc2_foundation::{NSDictionary, NSNumber, NSString, NSURL};

use super::MicrophoneAccess;

/// How many frames the engine is asked to hand over at once: 100 ms at 48 kHz. The engine may
/// choose another size.
const TAP_FRAMES: u32 = 4800;

pub fn access() -> MicrophoneAccess {
    // SAFETY: a constant of the framework, only read.
    let Some(audio) = (unsafe { AVMediaTypeAudio }) else { return MicrophoneAccess::Denied };
    // SAFETY: a plain query, with the media type the method takes.
    match unsafe { AVCaptureDevice::authorizationStatusForMediaType(audio) } {
        AVAuthorizationStatus::Authorized => MicrophoneAccess::Allowed,
        AVAuthorizationStatus::NotDetermined => MicrophoneAccess::Undetermined,
        _ => MicrophoneAccess::Denied,
    }
}

/// The system asks the user, once; `answered` runs with the answer on a queue of the system's.
pub fn ask(answered: impl FnOnce(bool) + Send + 'static) {
    // SAFETY: as above.
    let Some(audio) = (unsafe { AVMediaTypeAudio }) else {
        answered(false);
        return;
    };
    let answered = Mutex::new(Some(answered));
    let handler = RcBlock::new(move |granted: Bool| {
        let taken = answered.lock().ok().and_then(|mut slot| slot.take());
        if let Some(answered) = taken {
            answered(granted.as_bool());
        }
    });
    // SAFETY: AVFoundation copies the handler and calls it once.
    unsafe { AVCaptureDevice::requestAccessForMediaType_completionHandler(audio, &handler) };
}

pub fn open_settings() {
    let _ = std::process::Command::new("/usr/bin/open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")
        .spawn();
}

/// The microphone, listening for as long as this is kept; dropped, the engine stops.
pub struct Microphone {
    engine: Retained<AVAudioEngine>,
    // The tap's block, kept for as long as the tap is on (the engine holds a copy of its own).
    _tap: RcBlock<dyn Fn(NonNull<AVAudioPCMBuffer>, NonNull<AVAudioTime>)>,
}

impl Microphone {
    /// Start listening to the default input: `deliver` gets the sound, one channel, at the rate
    /// it is given with, on one of the engine's threads.
    pub fn start(deliver: impl Fn(&[f32], f64) + Send + Sync + 'static) -> Result<Microphone, String> {
        // SAFETY: a new engine; its input node is the system's default input device, and bus 0 is
        // that node's only one.
        let engine = unsafe { AVAudioEngine::new() };
        let input = unsafe { engine.inputNode() };
        let format = unsafe { input.outputFormatForBus(0) };
        // SAFETY: plain reads of the format.
        let (rate, channels) = unsafe { (format.sampleRate(), format.channelCount()) };
        if rate <= 0.0 || channels == 0 {
            return Err("the Mac has no microphone to listen to".into());
        }
        let tap = RcBlock::new(move |buffer: NonNull<AVAudioPCMBuffer>, _when: NonNull<AVAudioTime>| {
            // SAFETY: the engine hands over a buffer that is valid while the block runs.
            if let Some(samples) = unsafe { mono(buffer.as_ref()) } {
                deliver(&samples, rate);
            }
        });
        // SAFETY: no format (None) is the node's own; the engine copies the block.
        unsafe { input.installTapOnBus_bufferSize_format_block(0, TAP_FRAMES, None, RcBlock::as_ptr(&tap)) };
        // SAFETY: the engine has its input node and the tap; starting it may fail, and says why.
        unsafe { engine.prepare() };
        if let Err(err) = unsafe { engine.startAndReturnError() } {
            // SAFETY: the tap installed above.
            unsafe { input.removeTapOnBus(0) };
            return Err(err.localizedDescription().to_string());
        }
        Ok(Microphone { engine, _tap: tap })
    }
}

impl Drop for Microphone {
    fn drop(&mut self) {
        // SAFETY: the tap this struct installed, and the engine it started.
        unsafe {
            self.engine.inputNode().removeTapOnBus(0);
            self.engine.stop();
        }
    }
}

/// Core Audio's code for AAC ('aac ').
const AAC: u32 = 0x6161_6320;
/// A voice message's sound: one channel at 48 kHz, 48 kbit/s of AAC, which keeps speech clear at
/// about 6 kB a second.
const VOICE_RATE: f64 = 48_000.0;
const VOICE_BITS: i64 = 48_000;

/// A voice message being recorded into an M4A file; dropped without `finish`, the recording stops
/// and the file goes.
pub struct Recorder {
    recorder: Retained<AVAudioRecorder>,
    finished: bool,
}

impl Recorder {
    /// Start recording the default input into `path` (made, or written over).
    pub fn start(path: &std::path::Path) -> Result<Recorder, String> {
        // SAFETY: the framework's constants are only read.
        let keys = unsafe { [AVFormatIDKey, AVSampleRateKey, AVNumberOfChannelsKey, AVEncoderBitRateKey] };
        let [Some(format), Some(rate), Some(channels), Some(bits)] = keys else {
            return Err("AVFoundation has no recording settings".into());
        };
        let values = [NSNumber::new_u32(AAC), NSNumber::new_f64(VOICE_RATE), NSNumber::new_i64(1), NSNumber::new_i64(VOICE_BITS)];
        let objects: Vec<&AnyObject> = values.iter().map(|value| AsRef::<AnyObject>::as_ref(&**value)).collect();
        let settings = NSDictionary::<NSString, AnyObject>::from_slices(&[format, rate, channels, bits], &objects);
        let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        // SAFETY: a new recorder with settings of the keys the framework documents.
        let recorder = unsafe { AVAudioRecorder::initWithURL_settings_error(AVAudioRecorder::alloc(), &url, &settings) }
            .map_err(|err| err.localizedDescription().to_string())?;
        // SAFETY: plain calls on the recorder we own.
        unsafe {
            recorder.setMeteringEnabled(true);
            if !recorder.prepareToRecord() || !recorder.record() {
                return Err("the microphone did not start recording".into());
            }
        }
        Ok(Recorder { recorder, finished: false })
    }

    /// How loud the microphone is now, 0 … 1: 0 at about -50 dB, 1 at full scale.
    pub fn level(&self) -> f32 {
        // SAFETY: metering is on; channel 0 is the one channel.
        let decibels = unsafe {
            self.recorder.updateMeters();
            self.recorder.averagePowerForChannel(0)
        };
        ((decibels + 50.0) / 50.0).clamp(0.0, 1.0)
    }

    /// How long it has recorded, in seconds.
    pub fn seconds(&self) -> f64 {
        // SAFETY: a plain read.
        unsafe { self.recorder.currentTime() }
    }

    /// The recording stops and its file is complete; how long it is, in seconds.
    pub fn finish(mut self) -> f64 {
        let seconds = self.seconds();
        // SAFETY: stopping writes the file's end.
        unsafe { self.recorder.stop() };
        self.finished = true;
        seconds
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if !self.finished {
            // SAFETY: the recorder we own; its file goes with it.
            unsafe {
                self.recorder.stop();
                self.recorder.deleteRecording();
            }
        }
    }
}

/// The buffer's channels mixed down to one; none for a buffer that holds no floating-point sound.
///
/// # Safety
///
/// `buffer` must be one the engine handed over, while its block runs.
unsafe fn mono(buffer: &AVAudioPCMBuffer) -> Option<Vec<f32>> {
    // SAFETY: plain reads of the buffer and its format.
    let (frames, data, format, stride) = unsafe { (buffer.frameLength() as usize, buffer.floatChannelData(), buffer.format(), buffer.stride()) };
    if frames == 0 || data.is_null() {
        return None;
    }
    // SAFETY: as above.
    let (channels, interleaved) = unsafe { (format.channelCount() as usize, format.isInterleaved()) };
    let channels = channels.max(1);
    let stride = stride.max(1);
    let mut mixed = vec![0f32; frames];
    if interleaved {
        // One buffer holds every channel, a frame's samples side by side.
        // SAFETY: the first pointer is valid when the format is interleaved; the buffer holds
        // `frames` frames of `stride` samples.
        let samples = unsafe { (*data).as_ptr() };
        for (frame, out) in mixed.iter_mut().enumerate() {
            let sum: f32 = (0..channels.min(stride)).map(|channel| unsafe { *samples.add(frame * stride + channel) }).sum();
            *out = sum / channels.min(stride) as f32;
        }
    } else {
        for channel in 0..channels {
            // SAFETY: one pointer per channel, each to `frames` samples.
            let samples = unsafe { (*data.add(channel)).as_ptr() };
            for (frame, out) in mixed.iter_mut().enumerate() {
                *out += unsafe { *samples.add(frame) };
            }
        }
        let scale = 1.0 / channels as f32;
        for out in &mut mixed {
            *out *= scale;
        }
    }
    Some(mixed)
}
