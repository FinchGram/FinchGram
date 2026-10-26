//! The microphone's sound as the speech services take it: one channel of 16-bit little-endian PCM
//! at the service's rate (24 kHz for OpenAI's realtime sessions, 16 kHz for the others), in base64
//! for the JSON protocols, raw for Doubao's, in a WAV file for an upload (docs/voice-input.md).
//! The microphone gives floating point at its own rate: 48 or 44.1 kHz from a Mac's own, 16 or
//! 24 kHz through Bluetooth.

use base64::Engine as _;

/// Zero crossings of the low-pass's sinc on each side: how sharp its edge is.
const ZERO_CROSSINGS: f64 = 16.0;
/// Steps of the kernel's table per sample in: fine enough that reading between two steps is exact
/// to far below 16 bits.
const TABLE_STEPS: f64 = 512.0;

/// From the microphone's rate to the service's, a buffer at a time, band-limited: each sample out
/// is the samples in around its moment, weighted by a sinc that passes nothing above 92 % of the
/// lower rate's half, under a Blackman window. Nothing above the new rate's half folds back into
/// the speech, where the hiss of s, sh, x, z, c and q would otherwise land (the words right come
/// first, the maintainer's word of 2026-10-26). The same rate in and out passes as it is.
pub struct Resampler {
    /// How far one sample out moves in the samples in.
    step: f64,
    /// How far the kernel reaches on each side, in samples in.
    reach: usize,
    /// The kernel from 0 to `reach` samples in, TABLE_STEPS a sample.
    table: Vec<f32>,
    /// The samples in not yet left behind; the first `reach` were silence at the start.
    pending: Vec<f32>,
    /// Where the next sample out is, in `pending`.
    position: f64,
}

impl Resampler {
    /// For sound at `from` Hz, going out at `to` Hz.
    pub fn new(from: f64, to: u32) -> Resampler {
        let step = if from.is_finite() && from > 0.0 { from / f64::from(to.max(1)) } else { 1.0 };
        if (step - 1.0).abs() < 1e-9 {
            return Resampler { step: 1.0, reach: 0, table: Vec::new(), pending: Vec::new(), position: 0.0 };
        }
        // The cutoff, as a part of the rate in: below the lower of the two halves.
        let cutoff = 0.5 * (1.0 / step).min(1.0) * 0.92;
        let reach = (ZERO_CROSSINGS / (2.0 * cutoff)).ceil() as usize;
        let table = (0..=(reach as f64 * TABLE_STEPS) as usize + 1).map(|index| kernel(index as f64 / TABLE_STEPS, cutoff, reach as f64) as f32).collect();
        // Silence before the first sample, so that the first sample out is at the first sample in.
        Resampler { step, reach, table, pending: vec![0.0; reach], position: reach as f64 }
    }

    /// `input` in, at the microphone's rate; as many samples at the service's rate as it makes onto
    /// `out`. A sample out waits for `reach` samples in after it.
    pub fn push(&mut self, input: &[f32], out: &mut Vec<i16>) {
        if self.reach == 0 {
            out.extend(input.iter().map(|sample| pcm(*sample)));
            return;
        }
        self.pending.extend_from_slice(input);
        while self.position + self.reach as f64 + 1.0 <= self.pending.len() as f64 {
            out.push(pcm(self.sample_at(self.position)));
            self.position += self.step;
        }
        // What no sample out still needs goes.
        let behind = (self.position.floor() as usize).saturating_sub(self.reach);
        if behind > 0 {
            self.pending.drain(..behind);
            self.position -= behind as f64;
        }
    }

    /// The last samples out, which waited for sound after them: silence comes after the end.
    pub fn finish(&mut self, out: &mut Vec<i16>) {
        if self.reach > 0 {
            let silence = vec![0.0; self.reach + 1];
            self.push(&silence, out);
        }
    }

    /// The sound at `moment` (in samples of `pending`): the samples around it, weighted.
    fn sample_at(&self, moment: f64) -> f32 {
        let center = moment.floor() as isize;
        let reach = self.reach as isize;
        let mut sum = 0.0f32;
        for index in (center - reach + 1).max(0)..=(center + reach).min(self.pending.len() as isize - 1) {
            let distance = (moment - index as f64).abs() * TABLE_STEPS;
            let at = distance.floor() as usize;
            if at + 1 >= self.table.len() {
                continue;
            }
            let fraction = (distance - at as f64) as f32;
            let weight = self.table[at] * (1.0 - fraction) + self.table[at + 1] * fraction;
            sum += self.pending[index as usize] * weight;
        }
        sum
    }
}

/// The windowed sinc at `distance` samples in: a low-pass at `cutoff` (a part of the rate in),
/// under a Blackman window that reaches `reach` samples.
fn kernel(distance: f64, cutoff: f64, reach: f64) -> f64 {
    use std::f64::consts::PI;
    if distance.abs() >= reach {
        return 0.0;
    }
    let x = 2.0 * cutoff * distance;
    let sinc = if x.abs() < 1e-12 { 1.0 } else { (PI * x).sin() / (PI * x) };
    let window = 0.42 + 0.5 * (PI * distance / reach).cos() + 0.08 * (2.0 * PI * distance / reach).cos();
    2.0 * cutoff * sinc * window
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
    base64::engine::general_purpose::STANDARD.encode(bytes(samples))
}

/// Samples as little-endian bytes.
pub fn bytes(samples: &[i16]) -> Vec<u8> {
    samples.iter().flat_map(|sample| sample.to_le_bytes()).collect()
}

/// Samples at `rate` as a WAV file: one channel, 16-bit PCM.
pub fn wav(samples: &[i16], rate: u32) -> Vec<u8> {
    let data = u32::try_from(samples.len() * 2).unwrap_or(u32::MAX - 36);
    let mut file = Vec::with_capacity(44 + samples.len() * 2);
    file.extend_from_slice(b"RIFF");
    file.extend_from_slice(&(36 + data).to_le_bytes());
    file.extend_from_slice(b"WAVEfmt ");
    file.extend_from_slice(&16u32.to_le_bytes());
    file.extend_from_slice(&1u16.to_le_bytes()); // PCM
    file.extend_from_slice(&1u16.to_le_bytes()); // one channel
    file.extend_from_slice(&rate.to_le_bytes());
    file.extend_from_slice(&(rate * 2).to_le_bytes()); // bytes a second
    file.extend_from_slice(&2u16.to_le_bytes()); // bytes a sample
    file.extend_from_slice(&16u16.to_le_bytes()); // bits a sample
    file.extend_from_slice(b"data");
    file.extend_from_slice(&data.to_le_bytes());
    file.extend_from_slice(&bytes(samples));
    file
}

/// The samples of a WAV file of 16-bit PCM (what mpv writes, src/player/decode.rs): its data
/// chunk, as long as the file holds when the chunk's size was never written. Its first channel
/// only, should it have more.
pub fn wav_samples(file: &[u8]) -> Option<(Vec<i16>, u32)> {
    if file.len() < 12 || &file[0..4] != b"RIFF" || &file[8..12] != b"WAVE" {
        return None;
    }
    let (mut at, mut rate, mut channels, mut bits) = (12, 0u32, 1u16, 16u16);
    while at + 8 <= file.len() {
        let id = &file[at..at + 4];
        let size = u32::from_le_bytes(file[at + 4..at + 8].try_into().ok()?) as usize;
        let body = at + 8;
        if id == b"fmt " && body + 16 <= file.len() {
            channels = u16::from_le_bytes(file[body + 2..body + 4].try_into().ok()?).max(1);
            rate = u32::from_le_bytes(file[body + 4..body + 8].try_into().ok()?);
            bits = u16::from_le_bytes(file[body + 14..body + 16].try_into().ok()?);
        } else if id == b"data" {
            if bits != 16 || rate == 0 {
                return None;
            }
            let end = if size == 0 || body + size > file.len() { file.len() } else { body + size };
            let step = 2 * usize::from(channels);
            let samples = file[body..end].chunks_exact(step).map(|frame| i16::from_le_bytes([frame[0], frame[1]])).collect();
            return Some((samples, rate));
        }
        at = body + size + (size & 1);
    }
    None
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
        let mut resampler = Resampler::new(rate, 24_000);
        let mut out = Vec::new();
        for chunk in input.chunks(piece) {
            resampler.push(chunk, &mut out);
        }
        out
    }

    fn rms(samples: &[i16]) -> f64 {
        (samples.iter().map(|sample| f64::from(*sample).powi(2)).sum::<f64>() / samples.len().max(1) as f64).sqrt()
    }

    #[test]
    fn a_mac_microphones_rate_comes_down_to_24_khz_with_its_pitch_and_loudness() {
        let input = sine(48_000.0, 440.0, 1.0, 0.5);
        let mut out = resampled(48_000.0, &input, 4800);
        // The last samples wait for sound after them, until the end.
        assert!((23_940..24_000).contains(&out.len()), "{}", out.len());
        let mut resampler = Resampler::new(48_000.0, 24_000);
        out.clear();
        resampler.push(&input, &mut out);
        resampler.finish(&mut out);
        assert!((23_995..=24_001).contains(&out.len()), "{}", out.len());
        // 440 cycles in the second: 880 changes of sign.
        assert!((876..=884).contains(&sign_changes(&out)), "{}", sign_changes(&out));
        let peak = out[100..23_900].iter().map(|sample| sample.unsigned_abs()).max().unwrap_or(0);
        assert!((16_150..=16_600).contains(&peak), "{peak}");
        // 44.1 kHz too.
        let out = resampled(44_100.0, &sine(44_100.0, 440.0, 1.0, 0.5), 4410);
        assert!((23_940..24_000).contains(&out.len()), "{}", out.len());
        assert!((874..=884).contains(&sign_changes(&out)), "{}", sign_changes(&out));
    }

    #[test]
    fn nothing_above_the_new_half_folds_back_into_the_speech() {
        // A hiss at 10 kHz, above 16 kHz's half: gone, where the averaging of before let it
        // through at half its loudness, as a tone at 6 kHz.
        let hiss = resampled_16k(48_000.0, &sine(48_000.0, 10_000.0, 1.0, 0.5), 4800);
        let speech = resampled_16k(48_000.0, &sine(48_000.0, 3_000.0, 1.0, 0.5), 4800);
        let (hiss, speech) = (rms(&hiss[200..]), rms(&speech[200..]));
        assert!(speech > 11_000.0, "{speech}");
        assert!(hiss < speech / 300.0, "{hiss} against {speech}");
    }

    fn resampled_16k(rate: f64, input: &[f32], piece: usize) -> Vec<i16> {
        let mut resampler = Resampler::new(rate, 16_000);
        let mut out = Vec::new();
        for chunk in input.chunks(piece) {
            resampler.push(chunk, &mut out);
        }
        out
    }

    #[test]
    fn bluetooths_rate_goes_up_and_the_same_rate_stays_as_it_is() {
        let mut resampler = Resampler::new(16_000.0, 24_000);
        let mut out = Vec::new();
        resampler.push(&sine(16_000.0, 300.0, 1.0, 0.5), &mut out);
        resampler.finish(&mut out);
        assert!((23_995..=24_001).contains(&out.len()), "{}", out.len());
        assert!((596..=604).contains(&sign_changes(&out)), "{}", sign_changes(&out));
        let input = sine(24_000.0, 300.0, 0.1, 0.5);
        let out = resampled(24_000.0, &input, 480);
        assert_eq!(out.len(), input.len());
        assert!(out.iter().zip(&input).all(|(sample, original)| *sample == pcm(*original)));
    }

    #[test]
    fn the_sound_comes_out_the_same_whatever_the_pieces_it_came_in() {
        let input = sine(48_000.0, 523.0, 0.5, 0.3);
        let whole = resampled_16k(48_000.0, &input, input.len());
        assert_eq!(resampled_16k(48_000.0, &input, 4800), whole);
        assert_eq!(resampled_16k(48_000.0, &input, 997), whole);
        assert_eq!(resampled_16k(48_000.0, &input, 1), whole);
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
    fn sixteen_khz_for_the_other_services() {
        let mut resampler = Resampler::new(48_000.0, 16_000);
        let mut out = Vec::new();
        resampler.push(&sine(48_000.0, 440.0, 1.0, 0.5), &mut out);
        resampler.finish(&mut out);
        assert!((15_995..=16_001).contains(&out.len()), "{}", out.len());
        assert!((876..=884).contains(&sign_changes(&out)), "{}", sign_changes(&out));
    }

    #[test]
    fn a_wav_file_is_written_and_read_back() {
        let samples: Vec<i16> = (0..1000).map(|i| (i * 7 % 2000 - 1000) as i16).collect();
        let file = wav(&samples, 16_000);
        assert_eq!(file.len(), 44 + 2000);
        assert_eq!(&file[0..4], b"RIFF");
        assert_eq!(wav_samples(&file), Some((samples.clone(), 16_000)));
        // A data chunk whose size was never written (mpv's, cut short) holds what the file holds.
        let mut open = file.clone();
        open[40..44].copy_from_slice(&0x7fff_f000u32.to_le_bytes());
        assert_eq!(wav_samples(&open), Some((samples, 16_000)));
        assert_eq!(wav_samples(b"not a wav"), None);
    }

    #[test]
    fn samples_go_out_as_little_endian_base64() {
        assert_eq!(encode(&[1, -2]), "AQD+/w==");
        assert_eq!(encode(&[]), "");
        assert_eq!((pcm(2.0), pcm(-2.0), pcm(0.0)), (i16::MAX, -i16::MAX, 0));
    }
}
