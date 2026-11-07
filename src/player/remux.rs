//! A live stream as one stream mpv can read (docs/live-streams.md). Telegram sends a live stream
//! a second at a time, each second a complete MP4 file of its own (H.264 or HEVC, Opus or AAC),
//! and a chain of whole MP4 files is not something mpv plays as one. The samples are taken out of
//! each file and written into a single Matroska stream, as FFmpeg writes one into a pipe: the
//! segment of unknown size, then each second a cluster of its own, of known size.
//!
//! Nothing here touches the network or mpv: bytes in, bytes out (src/telegram/live.rs feeds it,
//! src/player/live.rs plays what comes out).

use std::fmt;

/// Matroska's clock: the timestamps below are milliseconds.
const TIMESTAMP_SCALE: u64 = 1_000_000;
/// How long an Opus decoder needs to settle after a jump, as Matroska wants it said (80 ms).
const OPUS_SEEK_PRE_ROLL: u64 = 80_000_000;
/// Opus always decodes at 48 kHz, whatever rate the source had.
const OPUS_RATE: u64 = 48_000;

/// Why a second of the stream could not be taken.
#[derive(Debug, Clone, PartialEq)]
pub enum RemuxError {
    /// Not an MP4 this module can read.
    Unreadable(String),
    /// Its tracks are not those the stream started with (another resolution, another codec):
    /// the stream has to start again, with a new header.
    Changed,
}

impl fmt::Display for RemuxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RemuxError::Unreadable(why) => write!(f, "unreadable MP4: {why}"),
            RemuxError::Changed => f.write_str("the stream's tracks changed"),
        }
    }
}

/// One stream being written: the tracks it started with, and whether its picture has started.
#[derive(Default)]
pub struct Remuxer {
    /// The stream's tracks, from its first second; none before.
    tracks: Option<Vec<OutTrack>>,
    /// The first key frame has been written: frames before it would decode into grey.
    picture_started: bool,
}

/// A track of the stream written: its number in it and what it carries.
#[derive(Clone, PartialEq)]
struct OutTrack {
    number: u8,
    codec: Codec,
}

impl Remuxer {
    pub fn new() -> Remuxer {
        Remuxer::default()
    }

    /// The bytes that carry one second of the stream, the MP4 file `mp4`, which begins
    /// `start_ms` after the stream began. The first second also brings the stream's header.
    pub fn push(&mut self, mp4: &[u8], start_ms: i64) -> Result<Vec<u8>, RemuxError> {
        let movie = parse(mp4).map_err(RemuxError::Unreadable)?;
        let mut tracks: Vec<&Track> = Vec::new();
        // One picture and one sound at most: the first of each that can be played.
        if let Some(video) = movie.tracks.iter().find(|track| track.codec.is_video()) {
            tracks.push(video);
        }
        if let Some(audio) = movie.tracks.iter().find(|track| !track.codec.is_video()) {
            tracks.push(audio);
        }
        if tracks.is_empty() {
            return Err(RemuxError::Unreadable("no track FinchGram can play".into()));
        }
        let mut out = Vec::new();
        // Each track's number in the stream: by its place, for the first second; afterwards the
        // number of the stream's track of the same kind, when it is the same stream (a second
        // may lack a track for a moment; a new kind, another resolution or codec, is another stream).
        let numbers: Vec<u8> = match &self.tracks {
            None => {
                let started: Vec<OutTrack> =
                    tracks.iter().enumerate().map(|(index, track)| OutTrack { number: index as u8 + 1, codec: track.codec.clone() }).collect();
                out.extend(header(&started));
                self.tracks = Some(started);
                (1..=tracks.len() as u8).collect()
            }
            Some(started) => {
                let mut numbers = Vec::with_capacity(tracks.len());
                for track in &tracks {
                    let same = started.iter().find(|out| out.codec.is_video() == track.codec.is_video()).filter(|out| out.codec.same_stream(&track.codec));
                    match same {
                        Some(out) => numbers.push(out.number),
                        None => return Err(RemuxError::Changed),
                    }
                }
                numbers
            }
        };

        // Every sample of the second, in the order they are decoded, as blocks of one cluster.
        let mut blocks: Vec<Block> = Vec::new();
        for (track, &number) in tracks.iter().zip(&numbers) {
            let video = track.codec.is_video();
            for sample in &track.samples {
                let present = track.presented(sample.cts);
                let decode = track.presented(sample.dts);
                if !video && present + track.ms(i64::from(sample.duration)) <= 0 {
                    // Sound from before the second, kept for the decoder by the edit list.
                    continue;
                }
                let data = &mp4[sample.offset..sample.offset + sample.size];
                blocks.push(Block { track: number, present, decode, key: !video || sample.sync, data });
            }
        }
        blocks.sort_by_key(|block| (block.decode, block.track));
        // The picture is track 1 whenever the stream has one.
        let has_video = self.tracks.as_ref().is_some_and(|started| started.first().is_some_and(|out| out.codec.is_video()));
        self.start_picture(&mut blocks, has_video);
        out.extend(cluster(start_ms, &blocks));
        Ok(out)
    }

    /// Until the stream's first key frame, its pictures are left out: a stream joined between key
    /// frames (the picture is track 1) starts at the next one, and its sound does not wait.
    fn start_picture(&mut self, blocks: &mut Vec<Block>, has_video: bool) {
        if self.picture_started {
            return;
        }
        if !has_video {
            self.picture_started = true;
            return;
        }
        match blocks.iter().position(|block| block.track == 1 && block.key) {
            Some(first_key) => {
                let mut index = 0;
                blocks.retain(|block| {
                    index += 1;
                    block.track != 1 || index > first_key
                });
                self.picture_started = true;
            }
            None => blocks.retain(|block| block.track != 1),
        }
    }
}

/// A sample on its way into a cluster: its time is milliseconds within the second.
struct Block<'a> {
    track: u8,
    present: i64,
    decode: i64,
    key: bool,
    data: &'a [u8],
}

// ---- reading an MP4 file -----------------------------------------------------------------------

/// What an MP4 file holds, as far as this module needs it.
struct Movie {
    tracks: Vec<Track>,
}

struct Track {
    codec: Codec,
    /// Units of the track's times in a second.
    timescale: u32,
    /// Where its presentation starts, in its own units (the edit list's media time).
    start: i64,
    samples: Vec<Sample>,
}

impl Track {
    /// A time of the track, in milliseconds from where its presentation starts.
    fn presented(&self, time: i64) -> i64 {
        self.ms(time - self.start)
    }

    fn ms(&self, time: i64) -> i64 {
        if self.timescale == 0 {
            return 0;
        }
        // Rounded to the nearest millisecond, the way Matroska keeps time.
        let scaled = i128::from(time) * 1000;
        let timescale = i128::from(self.timescale);
        ((scaled + timescale / 2).div_euclid(timescale)) as i64
    }
}

#[derive(Debug, Clone, Copy)]
struct Sample {
    offset: usize,
    size: usize,
    /// Decoding and presentation times, in the track's units.
    dts: i64,
    cts: i64,
    duration: u32,
    sync: bool,
}

/// What a track carries, with what its decoder needs to start.
#[derive(Debug, Clone, PartialEq)]
enum Codec {
    /// The avcC record, as Matroska wants it.
    H264 { config: Vec<u8>, width: u16, height: u16 },
    /// The hvcC record.
    Hevc { config: Vec<u8>, width: u16, height: u16 },
    /// The OpusHead Matroska wants, made from the dOps box; the samples to skip at the start.
    Opus { head: Vec<u8>, channels: u8, pre_skip: u16 },
    /// The AudioSpecificConfig of the esds box.
    Aac { config: Vec<u8>, channels: u16, rate: u32 },
}

impl Codec {
    fn is_video(&self) -> bool {
        matches!(self, Codec::H264 { .. } | Codec::Hevc { .. })
    }

    /// Whether a second's track carries on the stream's: the same picture (codec and parameters,
    /// so the same resolution), or sound with as many channels (Opus' pre-skip matters at the start
    /// only, and a muxer may write it differently each second).
    fn same_stream(&self, other: &Codec) -> bool {
        match (self, other) {
            (Codec::Opus { channels, .. }, Codec::Opus { channels: others, .. }) => channels == others,
            _ => self == other,
        }
    }
}

/// The tracks of `data`, an MP4 file whose moov describes its samples (not a fragmented one).
fn parse(data: &[u8]) -> Result<Movie, String> {
    let top = children(data)?;
    let moov = top.iter().find(|(kind, _)| kind == b"moov").map(|(_, body)| *body).ok_or("no moov box")?;
    let mut tracks = Vec::new();
    for (kind, trak) in children(moov)? {
        if &kind != b"trak" {
            continue;
        }
        // A track that cannot be read (an unknown codec, a hint track) is left out.
        if let Some(track) = parse_track(trak, data.len())? {
            tracks.push(track);
        }
    }
    Ok(Movie { tracks })
}

fn parse_track(trak: &[u8], file_size: usize) -> Result<Option<Track>, String> {
    let boxes = children(trak)?;
    let mdia = find(&boxes, b"mdia").ok_or("a track without mdia")?;
    let mdia = children(mdia)?;
    let mdhd = find(&mdia, b"mdhd").ok_or("a track without mdhd")?;
    let timescale = media_timescale(mdhd)?;
    let minf = children(find(&mdia, b"minf").ok_or("a track without minf")?)?;
    let stbl = children(find(&minf, b"stbl").ok_or("a track without stbl")?)?;
    let Some(codec) = sample_entry(find(&stbl, b"stsd").ok_or("a track without stsd")?)? else { return Ok(None) };
    let start = match find(&boxes, b"edts") {
        Some(edts) => match find(&children(edts)?, b"elst") {
            Some(elst) => edit_start(elst)?,
            None => 0,
        },
        None => 0,
    };
    let samples = sample_table(&stbl, file_size)?;
    Ok(Some(Track { codec, timescale, start, samples }))
}

/// Boxes by their four letters, with their bodies.
type Boxes<'a> = Vec<([u8; 4], &'a [u8])>;

/// The boxes inside `data`, by their four letters. A box of size 0 runs to the end.
fn children(data: &[u8]) -> Result<Boxes<'_>, String> {
    let mut boxes = Vec::new();
    let mut at = 0usize;
    while at + 8 <= data.len() {
        let size32 = u32::from_be_bytes(data[at..at + 4].try_into().unwrap_or_default());
        let kind: [u8; 4] = data[at + 4..at + 8].try_into().unwrap_or_default();
        let (size, header) = match size32 {
            0 => (data.len() - at, 8),
            1 => {
                let large = data.get(at + 8..at + 16).ok_or("a box cut short")?;
                (usize::try_from(u64::from_be_bytes(large.try_into().unwrap_or_default())).map_err(|_| "a box too large")?, 16)
            }
            size => (size as usize, 8),
        };
        if size < header || at.checked_add(size).is_none_or(|end| end > data.len()) {
            return Err(format!("the box {} runs past its parent", String::from_utf8_lossy(&kind)));
        }
        boxes.push((kind, &data[at + header..at + size]));
        at += size;
    }
    Ok(boxes)
}

fn find<'a>(boxes: &[([u8; 4], &'a [u8])], kind: &[u8; 4]) -> Option<&'a [u8]> {
    boxes.iter().find(|(found, _)| found == kind).map(|(_, body)| *body)
}

/// Big-endian numbers out of a box, each read checked against its end.
struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Reader<'a> {
        Reader { data, at: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], String> {
        let end = self.at.checked_add(count).filter(|end| *end <= self.data.len()).ok_or("a box cut short")?;
        let bytes = &self.data[self.at..end];
        self.at = end;
        Ok(bytes)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap_or_default()))
    }

    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap_or_default()))
    }

    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap_or_default()))
    }

    fn rest(&mut self) -> &'a [u8] {
        let rest = &self.data[self.at.min(self.data.len())..];
        self.at = self.data.len();
        rest
    }
}

fn media_timescale(mdhd: &[u8]) -> Result<u32, String> {
    let mut reader = Reader::new(mdhd);
    let version = reader.u8()?;
    reader.take(3)?;
    // Creation and modification times, then the timescale.
    reader.take(if version == 1 { 16 } else { 8 })?;
    let timescale = reader.u32()?;
    if timescale == 0 { Err("a track without a timescale".into()) } else { Ok(timescale) }
}

/// Where the track's presentation starts: the media time of its first edit that shows
/// something (an empty edit, -1, only delays it).
fn edit_start(elst: &[u8]) -> Result<i64, String> {
    let mut reader = Reader::new(elst);
    let version = reader.u8()?;
    reader.take(3)?;
    let count = reader.u32()?;
    for _ in 0..count {
        let media_time = if version == 1 {
            reader.u64()?;
            reader.u64()? as i64
        } else {
            reader.u32()?;
            i64::from(reader.u32()? as i32)
        };
        reader.take(4)?; // the rate
        if media_time >= 0 {
            return Ok(media_time);
        }
    }
    Ok(0)
}

/// The track's codec, from the first entry of its stsd box; none for one FinchGram cannot play.
fn sample_entry(stsd: &[u8]) -> Result<Option<Codec>, String> {
    let mut reader = Reader::new(stsd);
    reader.take(4)?; // version and flags
    if reader.u32()? == 0 {
        return Ok(None);
    }
    let entries = children(reader.rest())?;
    let Some((kind, entry)) = entries.first() else { return Ok(None) };
    match kind {
        b"avc1" | b"avc3" | b"hvc1" | b"hev1" => {
            // A visual sample entry: 24 bytes, its size, 50 more, then its boxes.
            let mut entry_reader = Reader::new(entry);
            entry_reader.take(24)?;
            let (width, height) = (entry_reader.u16()?, entry_reader.u16()?);
            entry_reader.take(50)?;
            let boxes = children(entry_reader.rest())?;
            if kind == b"avc1" || kind == b"avc3" {
                let config = find(&boxes, b"avcC").ok_or("an H.264 track without avcC")?;
                Ok(Some(Codec::H264 { config: config.to_vec(), width, height }))
            } else {
                let config = find(&boxes, b"hvcC").ok_or("an HEVC track without hvcC")?;
                Ok(Some(Codec::Hevc { config: config.to_vec(), width, height }))
            }
        }
        b"Opus" | b"mp4a" => {
            // An audio sample entry: 16 bytes, channels, sample size, 4 more, the rate (16.16),
            // then its boxes.
            let mut entry_reader = Reader::new(entry);
            entry_reader.take(16)?;
            let channels = entry_reader.u16()?;
            entry_reader.take(6)?;
            let rate = entry_reader.u32()? >> 16;
            let boxes = children(entry_reader.rest())?;
            if kind == b"Opus" {
                let dops = find(&boxes, b"dOps").ok_or("an Opus track without dOps")?;
                opus_head(dops).map(Some)
            } else {
                let esds = find(&boxes, b"esds").ok_or("an AAC track without esds")?;
                let config = audio_specific_config(esds)?;
                Ok(Some(Codec::Aac { config, channels, rate }))
            }
        }
        _ => Ok(None),
    }
}

/// The OpusHead Matroska carries (little-endian, as in Ogg), from the dOps box (big-endian).
fn opus_head(dops: &[u8]) -> Result<Codec, String> {
    let mut reader = Reader::new(dops);
    reader.u8()?; // version
    let channels = reader.u8()?;
    let pre_skip = reader.u16()?;
    let input_rate = reader.u32()?;
    let gain = reader.u16()?;
    let family = reader.u8()?;
    let mut head = Vec::with_capacity(19 + 2 + usize::from(channels));
    head.extend_from_slice(b"OpusHead");
    head.push(1);
    head.push(channels);
    head.extend_from_slice(&pre_skip.to_le_bytes());
    head.extend_from_slice(&input_rate.to_le_bytes());
    head.extend_from_slice(&gain.to_le_bytes());
    head.push(family);
    if family != 0 {
        head.push(reader.u8()?); // streams
        head.push(reader.u8()?); // coupled streams
        head.extend_from_slice(reader.take(usize::from(channels))?);
    }
    Ok(Codec::Opus { head, channels, pre_skip })
}

/// The AudioSpecificConfig inside an esds box: ES_Descriptor (3), DecoderConfigDescriptor (4),
/// DecoderSpecificInfo (5).
fn audio_specific_config(esds: &[u8]) -> Result<Vec<u8>, String> {
    let mut reader = Reader::new(esds);
    reader.take(4)?; // version and flags
    let mut tag = reader.u8()?;
    if tag != 3 {
        return Err("an esds without its ES descriptor".into());
    }
    descriptor_length(&mut reader)?;
    reader.take(2)?; // ES id
    let flags = reader.u8()?;
    if flags & 0x80 != 0 {
        reader.take(2)?;
    }
    if flags & 0x40 != 0 {
        let length = reader.u8()?;
        reader.take(usize::from(length))?;
    }
    if flags & 0x20 != 0 {
        reader.take(2)?;
    }
    tag = reader.u8()?;
    if tag != 4 {
        return Err("an esds without its decoder configuration".into());
    }
    descriptor_length(&mut reader)?;
    reader.take(13)?; // object type, stream type, buffer size, bit rates
    tag = reader.u8()?;
    if tag != 5 {
        return Err("an esds without the AAC configuration".into());
    }
    let length = descriptor_length(&mut reader)?;
    Ok(reader.take(length)?.to_vec())
}

/// A descriptor's length: up to four bytes of seven bits each, the high bit saying more follow.
fn descriptor_length(reader: &mut Reader) -> Result<usize, String> {
    let mut length = 0usize;
    for _ in 0..4 {
        let byte = reader.u8()?;
        length = (length << 7) | usize::from(byte & 0x7f);
        if byte & 0x80 == 0 {
            break;
        }
    }
    Ok(length)
}

/// Every sample of a track: where it is in the file, its times, whether it is a key frame.
fn sample_table(stbl: &[([u8; 4], &[u8])], file_size: usize) -> Result<Vec<Sample>, String> {
    // Sizes.
    let stsz = find(stbl, b"stsz").ok_or("a track without stsz")?;
    let mut reader = Reader::new(stsz);
    reader.take(4)?;
    let fixed = reader.u32()?;
    let count = reader.u32()? as usize;
    if count > 1_000_000 {
        return Err("too many samples for a second".into());
    }
    let mut sizes = Vec::with_capacity(count);
    for _ in 0..count {
        sizes.push(if fixed != 0 { fixed } else { reader.u32()? });
    }

    // Chunks: where they are, and how many samples each holds.
    let chunk_offsets: Vec<u64> = if let Some(stco) = find(stbl, b"stco") {
        let mut reader = Reader::new(stco);
        reader.take(4)?;
        let entries = reader.u32()?;
        (0..entries).map(|_| reader.u32().map(u64::from)).collect::<Result<_, _>>()?
    } else if let Some(co64) = find(stbl, b"co64") {
        let mut reader = Reader::new(co64);
        reader.take(4)?;
        let entries = reader.u32()?;
        (0..entries).map(|_| reader.u64()).collect::<Result<_, _>>()?
    } else {
        return Err("a track without stco or co64".into());
    };
    let stsc = find(stbl, b"stsc").ok_or("a track without stsc")?;
    let mut reader = Reader::new(stsc);
    reader.take(4)?;
    let runs: Vec<(u32, u32)> = (0..reader.u32()?)
        .map(|_| {
            let first_chunk = reader.u32()?;
            let per_chunk = reader.u32()?;
            reader.u32()?; // the sample description: one per track here
            Ok((first_chunk, per_chunk))
        })
        .collect::<Result<_, String>>()?;
    let mut offsets = Vec::with_capacity(count);
    for (index, &(first_chunk, per_chunk)) in runs.iter().enumerate() {
        let last_chunk = runs.get(index + 1).map_or(chunk_offsets.len() as u32, |next| next.0.saturating_sub(1));
        for chunk in first_chunk.max(1)..=last_chunk {
            let Some(&chunk_offset) = chunk_offsets.get(chunk as usize - 1) else { break };
            let mut offset = chunk_offset;
            for _ in 0..per_chunk {
                let Some(&size) = sizes.get(offsets.len()) else { break };
                offsets.push(offset);
                offset += u64::from(size);
            }
        }
    }
    if offsets.len() < count {
        return Err("the chunks hold fewer samples than stsz says".into());
    }

    // Times.
    let stts = find(stbl, b"stts").ok_or("a track without stts")?;
    let mut reader = Reader::new(stts);
    reader.take(4)?;
    let mut durations = Vec::with_capacity(count);
    for _ in 0..reader.u32()? {
        let (samples, delta) = (reader.u32()?, reader.u32()?);
        for _ in 0..samples {
            if durations.len() == count {
                break;
            }
            durations.push(delta);
        }
    }
    durations.resize(count, durations.last().copied().unwrap_or(0));
    let mut offsets_in_time = vec![0i64; count];
    if let Some(ctts) = find(stbl, b"ctts") {
        let mut reader = Reader::new(ctts);
        reader.take(4)?;
        let mut index = 0;
        for _ in 0..reader.u32()? {
            let samples = reader.u32()?;
            // Signed in version 1, and in practice in version 0 too (as FFmpeg reads them).
            let offset = i64::from(reader.u32()? as i32);
            for _ in 0..samples {
                if index == count {
                    break;
                }
                offsets_in_time[index] = offset;
                index += 1;
            }
        }
    }
    let sync: Option<Vec<u32>> = match find(stbl, b"stss") {
        Some(stss) => {
            let mut reader = Reader::new(stss);
            reader.take(4)?;
            Some((0..reader.u32()?).map(|_| reader.u32()).collect::<Result<_, _>>()?)
        }
        None => None,
    };

    let mut samples = Vec::with_capacity(count);
    let mut dts = 0i64;
    for index in 0..count {
        let (offset, size) = (offsets[index], sizes[index] as usize);
        let offset = usize::try_from(offset).map_err(|_| "a sample past the file")?;
        if offset.checked_add(size).is_none_or(|end| end > file_size) {
            return Err("a sample past the end of the file".into());
        }
        let is_sync = sync.as_ref().is_none_or(|numbers| numbers.binary_search(&(index as u32 + 1)).is_ok());
        samples.push(Sample { offset, size, dts, cts: dts + offsets_in_time[index], duration: durations[index], sync: is_sync });
        dts += i64::from(durations[index]);
    }
    Ok(samples)
}

// ---- writing Matroska --------------------------------------------------------------------------

mod id {
    pub const EBML: u32 = 0x1A45_DFA3;
    pub const EBML_VERSION: u32 = 0x4286;
    pub const EBML_READ_VERSION: u32 = 0x42F7;
    pub const EBML_MAX_ID_LENGTH: u32 = 0x42F2;
    pub const EBML_MAX_SIZE_LENGTH: u32 = 0x42F3;
    pub const DOC_TYPE: u32 = 0x4282;
    pub const DOC_TYPE_VERSION: u32 = 0x4287;
    pub const DOC_TYPE_READ_VERSION: u32 = 0x4285;
    pub const SEGMENT: u32 = 0x1853_8067;
    pub const INFO: u32 = 0x1549_A966;
    pub const TIMESTAMP_SCALE: u32 = 0x2A_D7B1;
    pub const MUXING_APP: u32 = 0x4D80;
    pub const WRITING_APP: u32 = 0x5741;
    pub const TRACKS: u32 = 0x1654_AE6B;
    pub const TRACK_ENTRY: u32 = 0xAE;
    pub const TRACK_NUMBER: u32 = 0xD7;
    pub const TRACK_UID: u32 = 0x73C5;
    pub const TRACK_TYPE: u32 = 0x83;
    pub const FLAG_LACING: u32 = 0x9C;
    pub const CODEC_ID: u32 = 0x86;
    pub const CODEC_PRIVATE: u32 = 0x63A2;
    pub const CODEC_DELAY: u32 = 0x56AA;
    pub const SEEK_PRE_ROLL: u32 = 0x56BB;
    pub const VIDEO: u32 = 0xE0;
    pub const PIXEL_WIDTH: u32 = 0xB0;
    pub const PIXEL_HEIGHT: u32 = 0xBA;
    pub const AUDIO: u32 = 0xE1;
    pub const SAMPLING_FREQUENCY: u32 = 0xB5;
    pub const CHANNELS: u32 = 0x9F;
    pub const CLUSTER: u32 = 0x1F43_B675;
    pub const TIMESTAMP: u32 = 0xE7;
    pub const SIMPLE_BLOCK: u32 = 0xA3;
}

/// The stream's beginning: its EBML header, the segment (of unknown size: it goes on while the
/// live stream does), its info and its tracks.
fn header(tracks: &[OutTrack]) -> Vec<u8> {
    let mut out = element(
        id::EBML,
        &[
            uint(id::EBML_VERSION, 1),
            uint(id::EBML_READ_VERSION, 1),
            uint(id::EBML_MAX_ID_LENGTH, 4),
            uint(id::EBML_MAX_SIZE_LENGTH, 8),
            string(id::DOC_TYPE, "matroska"),
            uint(id::DOC_TYPE_VERSION, 4),
            uint(id::DOC_TYPE_READ_VERSION, 2),
        ]
        .concat(),
    );
    out.extend(id_bytes(id::SEGMENT));
    out.extend([0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
    out.extend(element(
        id::INFO,
        &[uint(id::TIMESTAMP_SCALE, TIMESTAMP_SCALE), string(id::MUXING_APP, "FinchGram"), string(id::WRITING_APP, "FinchGram")].concat(),
    ));
    let entries: Vec<u8> = tracks.iter().flat_map(track_entry).collect();
    out.extend(element(id::TRACKS, &entries));
    out
}

fn track_entry(track: &OutTrack) -> Vec<u8> {
    let number = u64::from(track.number);
    let mut fields = vec![uint(id::TRACK_NUMBER, number), uint(id::TRACK_UID, number), uint(id::FLAG_LACING, 0)];
    match &track.codec {
        Codec::H264 { config, width, height } | Codec::Hevc { config, width, height } => {
            let codec_id = if matches!(track.codec, Codec::H264 { .. }) { "V_MPEG4/ISO/AVC" } else { "V_MPEGH/ISO/HEVC" };
            fields.extend([
                uint(id::TRACK_TYPE, 1),
                string(id::CODEC_ID, codec_id),
                element(id::CODEC_PRIVATE, config),
                element(id::VIDEO, &[uint(id::PIXEL_WIDTH, u64::from(*width)), uint(id::PIXEL_HEIGHT, u64::from(*height))].concat()),
            ]);
        }
        Codec::Opus { head, channels, pre_skip } => {
            fields.extend([
                uint(id::TRACK_TYPE, 2),
                string(id::CODEC_ID, "A_OPUS"),
                element(id::CODEC_PRIVATE, head),
                uint(id::CODEC_DELAY, u64::from(*pre_skip) * 1_000_000_000 / OPUS_RATE),
                uint(id::SEEK_PRE_ROLL, OPUS_SEEK_PRE_ROLL),
                element(id::AUDIO, &[float(id::SAMPLING_FREQUENCY, OPUS_RATE as f64), uint(id::CHANNELS, u64::from(*channels))].concat()),
            ]);
        }
        Codec::Aac { config, channels, rate } => {
            fields.extend([
                uint(id::TRACK_TYPE, 2),
                string(id::CODEC_ID, "A_AAC"),
                element(id::CODEC_PRIVATE, config),
                element(id::AUDIO, &[float(id::SAMPLING_FREQUENCY, f64::from(*rate)), uint(id::CHANNELS, u64::from(*channels))].concat()),
            ]);
        }
    }
    element(id::TRACK_ENTRY, &fields.concat())
}

/// One second as a cluster: its timestamp, then its samples as simple blocks, each at its time
/// within the cluster.
fn cluster(start_ms: i64, blocks: &[Block]) -> Vec<u8> {
    let mut body = uint(id::TIMESTAMP, u64::try_from(start_ms).unwrap_or(0));
    for block in blocks {
        let relative = block.present.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16;
        let mut payload = Vec::with_capacity(block.data.len() + 4);
        payload.push(0x80 | block.track);
        payload.extend_from_slice(&relative.to_be_bytes());
        payload.push(if block.key { 0x80 } else { 0x00 });
        payload.extend_from_slice(block.data);
        body.extend(element(id::SIMPLE_BLOCK, &payload));
    }
    element(id::CLUSTER, &body)
}

fn element(id: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = id_bytes(id);
    out.extend(size_bytes(payload.len() as u64));
    out.extend_from_slice(payload);
    out
}

fn uint(id: u32, value: u64) -> Vec<u8> {
    let bytes = value.to_be_bytes();
    let first = bytes.iter().position(|&byte| byte != 0).unwrap_or(7);
    element(id, &bytes[first..])
}

fn float(id: u32, value: f64) -> Vec<u8> {
    element(id, &value.to_be_bytes())
}

fn string(id: u32, value: &str) -> Vec<u8> {
    element(id, value.as_bytes())
}

/// An element id as written: its bytes carry their own length marker already.
fn id_bytes(id: u32) -> Vec<u8> {
    let bytes = id.to_be_bytes();
    let first = bytes.iter().position(|&byte| byte != 0).unwrap_or(3);
    bytes[first..].to_vec()
}

/// A size as EBML writes it: the fewest bytes, their leading bits saying how many (a size of all
/// ones is reserved for "unknown").
fn size_bytes(size: u64) -> Vec<u8> {
    for length in 1..=8u32 {
        if size < (1u64 << (7 * length)) - 1 {
            let marked = size | (1u64 << (7 * length));
            return marked.to_be_bytes()[(8 - length) as usize..].to_vec();
        }
    }
    // Larger than any second of video; written as eight bytes all the same.
    let marked = size | (1u64 << 56);
    marked.to_be_bytes().to_vec()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const SECONDS: [&[u8]; 3] = [
        include_bytes!("fixtures/live-second-0.mp4"),
        include_bytes!("fixtures/live-second-1.mp4"),
        include_bytes!("fixtures/live-second-2.mp4"),
    ];

    #[test]
    fn a_second_of_video_and_sound_is_read() {
        let movie = parse(SECONDS[1]).expect("readable");
        assert_eq!(movie.tracks.len(), 2);
        let video = &movie.tracks[0];
        let Codec::H264 { config, width, height } = &video.codec else { panic!("not H.264: {:?}", video.codec) };
        assert_eq!((*width, *height), (160, 90));
        assert_eq!(config[0], 1, "an avcC record starts with its version");
        assert_eq!(video.samples.len(), 30);
        assert!(video.samples[0].sync && !video.samples[1].sync);
        assert_eq!(video.timescale, 16000);
        assert_eq!(video.start, 1072, "the edit list's media time");
        // B-frames: presented later than decoded.
        assert!(video.samples.iter().any(|sample| sample.cts != sample.dts));
        let audio = &movie.tracks[1];
        let Codec::Opus { head, channels, pre_skip } = &audio.codec else { panic!("not Opus: {:?}", audio.codec) };
        assert_eq!(&head[..8], b"OpusHead");
        assert_eq!(*channels, 2);
        assert_eq!(u16::from_le_bytes([head[10], head[11]]), *pre_skip);
        assert_eq!(audio.samples.len(), 50);
        // Every sample lies inside the file.
        for sample in video.samples.iter().chain(&audio.samples) {
            assert!(sample.offset + sample.size <= SECONDS[1].len());
        }
    }

    #[test]
    fn sizes_are_written_in_the_fewest_bytes() {
        assert_eq!(size_bytes(0), vec![0x80]);
        assert_eq!(size_bytes(126), vec![0xFE]);
        // 127 would be all ones in one byte, which means "unknown".
        assert_eq!(size_bytes(127), vec![0x40, 0x7F]);
        assert_eq!(size_bytes(0x3FFE), vec![0x7F, 0xFE]);
        assert_eq!(size_bytes(0x3FFF), vec![0x20, 0x3F, 0xFF]);
        assert_eq!(id_bytes(id::EBML), vec![0x1A, 0x45, 0xDF, 0xA3]);
        assert_eq!(id_bytes(id::SIMPLE_BLOCK), vec![0xA3]);
        assert_eq!(uint(id::TRACK_NUMBER, 1), vec![0xD7, 0x81, 0x01]);
        assert_eq!(uint(id::TIMESTAMP, 0), vec![0xE7, 0x81, 0x00]);
    }

    #[test]
    fn the_first_second_brings_the_header_and_the_others_only_their_cluster() {
        let mut remuxer = Remuxer::new();
        let first = remuxer.push(SECONDS[1], 0).expect("first");
        assert_eq!(&first[..4], &[0x1A, 0x45, 0xDF, 0xA3]);
        let second = remuxer.push(SECONDS[2], 1000).expect("second");
        assert_eq!(&second[..4], &[0x1F, 0x43, 0xB6, 0x75], "a cluster, no header");
    }

    #[test]
    fn a_second_without_a_track_or_with_another_pre_skip_carries_on() {
        let movie = parse(SECONDS[1]).expect("readable");
        let video = OutTrack { number: 1, codec: movie.tracks[0].codec.clone() };
        let Codec::Opus { head, channels, .. } = movie.tracks[1].codec.clone() else { panic!("not Opus") };
        // The stream began with another pre-skip in its Opus header.
        let audio = OutTrack { number: 2, codec: Codec::Opus { head, channels, pre_skip: 0 } };
        let mut remuxer = Remuxer { tracks: Some(vec![video, audio]), picture_started: true };
        let bytes = remuxer.push(SECONDS[1], 0).expect("the same stream");
        let cluster = elements(&bytes)[0].1;
        let tracks: std::collections::HashSet<u8> = elements(cluster).iter().filter(|(id, _)| *id == id::SIMPLE_BLOCK).map(|(_, block)| block[0]).collect();
        assert_eq!(tracks, [0x81, 0x82].into_iter().collect());
        // A stream of sound alone gets a second with a picture: another stream.
        let mut sound_only = Remuxer { tracks: Some(vec![OutTrack { number: 1, codec: movie.tracks[1].codec.clone() }]), picture_started: true };
        assert_eq!(sound_only.push(SECONDS[1], 0).err(), Some(RemuxError::Changed));
    }

    #[test]
    fn a_stream_whose_tracks_change_has_to_start_again() {
        // A stream that began with another picture (another resolution): another stream.
        let movie = parse(SECONDS[1]).expect("readable");
        let Codec::H264 { config, .. } = movie.tracks[0].codec.clone() else { panic!("not H.264") };
        let other = Codec::H264 { config, width: 1280, height: 720 };
        let mut remuxer = Remuxer { tracks: Some(vec![OutTrack { number: 1, codec: other }, OutTrack { number: 2, codec: movie.tracks[1].codec.clone() }]), picture_started: true };
        assert_eq!(remuxer.push(SECONDS[1], 0).err(), Some(RemuxError::Changed));
        assert!(matches!(Remuxer::new().push(b"not an mp4 at all", 0), Err(RemuxError::Unreadable(_))));
    }

    /// The elements one after another in `bytes`: id, payload. A size of all ones (unknown) runs
    /// to the end.
    pub(crate) fn elements(bytes: &[u8]) -> Vec<(u32, &[u8])> {
        let mut found = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            let id_length = bytes[at].leading_zeros() as usize + 1;
            let id = bytes[at..at + id_length].iter().fold(0u32, |id, &byte| (id << 8) | u32::from(byte));
            at += id_length;
            let size_length = bytes[at].leading_zeros() as usize + 1;
            let mut size = u64::from(bytes[at]) & (0xFFu64 >> size_length);
            for &byte in &bytes[at + 1..at + size_length] {
                size = (size << 8) | u64::from(byte);
            }
            let unknown = size == (1u64 << (7 * size_length)) - 1;
            at += size_length;
            let end = if unknown { bytes.len() } else { at + size as usize };
            found.push((id, &bytes[at..end]));
            at = end;
        }
        found
    }

    #[test]
    fn the_stream_is_matroska_with_a_cluster_a_second() {
        let mut remuxer = Remuxer::new();
        let mut stream = Vec::new();
        for (index, second) in SECONDS[1..].iter().enumerate() {
            stream.extend(remuxer.push(second, index as i64 * 1000).expect("a second"));
        }
        let top = elements(&stream);
        assert_eq!(top.iter().map(|(id, _)| *id).collect::<Vec<_>>(), vec![id::EBML, id::SEGMENT]);
        let segment = elements(top[1].1);
        let ids: Vec<u32> = segment.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, vec![id::INFO, id::TRACKS, id::CLUSTER, id::CLUSTER]);
        let tracks = elements(segment[1].1);
        assert_eq!(tracks.len(), 2, "a picture and a sound");
        let codecs: Vec<String> = tracks
            .iter()
            .map(|(_, entry)| {
                let codec = elements(entry).into_iter().find(|(id, _)| *id == id::CODEC_ID).expect("a codec").1;
                String::from_utf8_lossy(codec).into_owned()
            })
            .collect();
        assert_eq!(codecs, vec!["V_MPEG4/ISO/AVC", "A_OPUS"]);
        for (index, (_, cluster)) in segment[2..].iter().enumerate() {
            let children = elements(cluster);
            assert_eq!(children[0].0, id::TIMESTAMP);
            let timestamp = children[0].1.iter().fold(0u64, |value, &byte| (value << 8) | u64::from(byte));
            assert_eq!(timestamp, index as u64 * 1000);
            let blocks: Vec<&[u8]> = children[1..].iter().filter(|(id, _)| *id == id::SIMPLE_BLOCK).map(|(_, block)| *block).collect();
            assert_eq!(blocks.iter().filter(|block| block[0] == 0x81).count(), 30, "30 pictures a second");
            let sound = blocks.iter().filter(|block| block[0] == 0x82).count();
            assert!((45..=50).contains(&sound), "about 50 Opus frames a second, not {sound}");
            // The picture's first block is a key frame, presented at the start of the second.
            let first_picture = blocks.iter().find(|block| block[0] == 0x81).expect("a picture");
            assert_eq!(first_picture[3] & 0x80, 0x80);
            assert_eq!(i16::from_be_bytes([first_picture[1], first_picture[2]]), 0);
        }
    }

    #[test]
    fn a_stream_joined_between_key_frames_starts_its_picture_at_the_next() {
        let block = |track: u8, key: bool| Block { track, present: 0, decode: 0, key, data: &[] };
        let mut remuxer = Remuxer::new();
        // A second with no key frame: its pictures wait, its sound plays.
        let mut blocks = vec![block(1, false), block(2, true), block(1, false)];
        remuxer.start_picture(&mut blocks, true);
        assert_eq!(blocks.iter().map(|block| block.track).collect::<Vec<_>>(), vec![2]);
        assert!(!remuxer.picture_started);
        // The next one has a key frame partway: the picture starts there.
        let mut blocks = vec![block(1, false), block(2, true), block(1, true), block(1, false), block(2, true)];
        remuxer.start_picture(&mut blocks, true);
        assert_eq!(blocks.iter().map(|block| (block.track, block.key)).collect::<Vec<_>>(), vec![(2, true), (1, true), (1, false), (2, true)]);
        assert!(remuxer.picture_started);
        // From then on nothing is left out.
        let mut blocks = vec![block(1, false), block(2, true)];
        remuxer.start_picture(&mut blocks, true);
        assert_eq!(blocks.len(), 2);
    }
}
