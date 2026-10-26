//! The microphone's sound as OpenAI's realtime transcription takes it: one channel at 24 kHz,
//! 16-bit little-endian PCM, in base64 (docs/voice-input.md). The microphone gives floating point
//! at its own rate: 48 or 44.1 kHz from a Mac's own, 16 or 24 kHz through Bluetooth.

use base64::Engine as _;

/// The rate OpenAI's realtime sessions take.
pub const RATE: u32 = 24_000;

/// From the microphone's rate to RATE, a buffer at a time: linear interpolation, after an average
/// over about one step when going down, so that what is above the new rate's half does not fold
/// back into the speech.
pub struct Resampler {
    /// How far one sample out moves in the samples in.
    step: f64,
    /// Where the next sample out is, in `pending`.
    position: f64,
    /// The samples in, averaged, not used up yet.
    pending: Vec<f32>,
    /// How many samples in each average takes, and the last ones of the buffer before, which the
    /// next averages still need.
    width: usize,
    history: Vec<f32>,
}

impl Resampler {
    /// For sound at `from` Hz.
    pub fn new(from: f64) -> Resampler {
        let step = if from.is_finite() && from > 0.0 { from / f64::from(RATE) } else { 1.0 };
        let width = if step >= 1.5 { step.round() as usize } else { 1 };
        Resampler { step, position: 0.0, pending: Vec::new(), width, history: Vec::new() }
    }

    /// `input` in, at the microphone's rate; as many samples at RATE as it makes onto `out`.
    pub fn push(&mut self, input: &[f32], out: &mut Vec<i16>) {
        if self.width > 1 {
            let mut window = std::mem::take(&mut self.history);
            window.extend_from_slice(input);
            let width = self.width;
            if window.len() >= width {
                for end in width..=window.len() {
                    let sum: f32 = window[end - width..end].iter().sum();
                    self.pending.push(sum / width as f32);
                }
                self.history = window[window.len() + 1 - width..].to_vec();
            } else {
                self.history = window;
            }
        } else {
            self.pending.extend_from_slice(input);
        }
        while self.position + 1.0 < self.pending.len() as f64 {
            let index = self.position.floor() as usize;
            let fraction = (self.position - index as f64) as f32;
            let sample = self.pending[index] * (1.0 - fraction) + self.pending[index + 1] * fraction;
            out.push(pcm(sample));
            self.position += self.step;
        }
        let used = (self.position.floor() as usize).min(self.pending.len());
        self.pending.drain(..used);
        self.position -= used as f64;
    }
}

/// A sample of -1 … 1 as 16 bits.
fn pcm(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16
}

/// How loud `samples` are, for the bars: 0 at about -55 dB below full scale (a quiet room), 1 at
/// -15 dB (speech close to the microphone).
pub fn level(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let mean_square = samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32;
    if mean_square <= f32::MIN_POSITIVE {
        return 0.0;
    }
    ((10.0 * mean_square.log10() + 55.0) / 40.0).clamp(0.0, 1.0)
}

/// Samples as the events carry them: little-endian bytes, in base64.
pub fn encode(samples: &[i16]) -> String {
    let bytes: Vec<u8> = samples.iter().flat_map(|sample| sample.to_le_bytes()).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: f64, frequency: f64, seconds: f64, amplitude: f32) -> Vec<f32> {
        (0..(rate * seconds) as usize).map(|i| amplitude * (2.0 * std::f64::consts::PI * frequency * i as f64 / rate).sin() as f32).collect()
    }

    fn sign_changes(samples: &[i16]) -> usize {
        samples.windows(2).filter(|pair| (pair[0] < 0) != (pair[1] < 0)).count()
    }

    fn resampled(rate: f64, input: &[f32], piece: usize) -> Vec<i16> {
        let mut resampler = Resampler::new(rate);
        let mut out = Vec::new();
        for chunk in input.chunks(piece) {
            resampler.push(chunk, &mut out);
        }
        out
    }

    #[test]
    fn a_mac_microphones_rate_comes_down_to_24_khz_with_its_pitch_and_loudness() {
        let input = sine(48_000.0, 440.0, 1.0, 0.5);
        let out = resampled(48_000.0, &input, 4800);
        assert!((23_990..=24_000).contains(&out.len()), "{}", out.len());
        // 440 cycles in the second: 880 changes of sign.
        assert!((876..=884).contains(&sign_changes(&out)), "{}", sign_changes(&out));
        let peak = out.iter().map(|sample| sample.unsigned_abs()).max().unwrap_or(0);
        assert!((16_000..=16_400).contains(&peak), "{peak}");
        // 44.1 kHz too.
        let out = resampled(44_100.0, &sine(44_100.0, 440.0, 1.0, 0.5), 4410);
        assert!((23_990..=24_000).contains(&out.len()), "{}", out.len());
        assert!((876..=884).contains(&sign_changes(&out)), "{}", sign_changes(&out));
    }

    #[test]
    fn bluetooths_rate_goes_up_and_24_khz_stays_as_it_is() {
        let out = resampled(16_000.0, &sine(16_000.0, 300.0, 1.0, 0.5), 1600);
        assert!((23_980..=24_000).contains(&out.len()), "{}", out.len());
        assert!((596..=604).contains(&sign_changes(&out)), "{}", sign_changes(&out));
        let input = sine(24_000.0, 300.0, 0.1, 0.5);
        let out = resampled(24_000.0, &input, 480);
        assert_eq!(out.len(), input.len() - 1);
        assert!(out.iter().zip(&input).all(|(sample, original)| *sample == pcm(*original)));
    }

    #[test]
    fn the_sound_comes_out_the_same_whatever_the_pieces_it_came_in() {
        let input = sine(48_000.0, 523.0, 0.5, 0.3);
        let whole = resampled(48_000.0, &input, input.len());
        assert_eq!(resampled(48_000.0, &input, 4800), whole);
        assert_eq!(resampled(48_000.0, &input, 997), whole);
        assert_eq!(resampled(48_000.0, &input, 1), whole);
    }

    #[test]
    fn loudness_goes_from_a_quiet_room_to_speech_up_close() {
        assert_eq!(level(&[]), 0.0);
        assert_eq!(level(&[0.0; 480]), 0.0);
        assert_eq!(level(&[1.0, -1.0, 1.0, -1.0]), 1.0);
        // -35 dB is halfway.
        let amplitude = 10f32.powf(-35.0 / 20.0);
        assert!((level(&[amplitude, -amplitude]) - 0.5).abs() < 0.01);
        assert_eq!(level(&[0.0005; 100]), 0.0);
    }

    #[test]
    fn samples_go_out_as_little_endian_base64() {
        assert_eq!(encode(&[1, -2]), "AQD+/w==");
        assert_eq!(encode(&[]), "");
        assert_eq!((pcm(2.0), pcm(-2.0), pcm(0.0)), (i16::MAX, -i16::MAX, 0));
    }
}
