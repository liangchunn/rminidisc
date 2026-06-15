//! Audio decoding and normalization used by the `rminidisc` upload command.
//!
//! This crate intentionally knows nothing about MiniDisc wire formats or ATRAC
//! encoding. It only replaces the old FFmpeg normalization steps:
//!
//! - `-ac 2 -ar 44100 -f s16be`  → [decode_to_s16be_44100_stereo]
//! - `-ac 2 -ar 44100 -f wav`    → [decode_to_wav_44100_stereo]

pub(crate) mod decoder;
pub mod error;
pub(crate) mod output;
pub(crate) mod resample;

use std::io::{Seek, Write};
use std::path::Path;

use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

use crate::decoder::decode_to_44100_stereo_streaming;
use crate::error::Result;
use crate::output::{S16beSink, WavSink};

/// Decode `path`, normalize it to stereo 44.1 kHz PCM, and stream interleaved
/// signed 16-bit big-endian samples to `writer`.
///
/// Memory use is bounded by the decoder/resampler chunk sizes, not the track
/// length: decoded audio is converted and written incrementally.
pub fn decode_to_s16be_44100_stereo_writer<W: Write>(
    path: impl AsRef<Path>,
    writer: W,
) -> Result<()> {
    let mut sink = S16beSink::new(writer);
    decode_to_44100_stereo_streaming(path.as_ref(), &mut sink)
}

/// Decode `path`, normalize it to stereo 44.1 kHz PCM, and stream a 16-bit PCM
/// WAV file to `writer`. The writer must be seekable so the WAV header can be
/// patched on finalize.
pub fn decode_to_wav_44100_stereo_writer<W: Write + Seek>(
    path: impl AsRef<Path>,
    writer: W,
) -> Result<()> {
    let mut sink = WavSink::new(writer)?;
    decode_to_44100_stereo_streaming(path.as_ref(), &mut sink)
}

/// Decode `path`, normalize it to stereo 44.1 kHz PCM, and return interleaved
/// signed 16-bit big-endian samples.
///
/// Prefer [`decode_to_s16be_44100_stereo_writer`] for large inputs; this buffers
/// the entire result in memory.
pub fn decode_to_s16be_44100_stereo(path: impl AsRef<Path>) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    decode_to_s16be_44100_stereo_writer(path, &mut out)?;
    Ok(out)
}

/// Decode `path`, normalize it to stereo 44.1 kHz PCM, and return a WAV file
/// containing signed 16-bit little-endian PCM samples.
///
/// Prefer [`decode_to_wav_44100_stereo_writer`] for large inputs; this buffers
/// the entire result in memory.
pub fn decode_to_wav_44100_stereo(path: impl AsRef<Path>) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    decode_to_wav_44100_stereo_writer(path, std::io::Cursor::new(&mut out))?;
    Ok(out)
}

/// Lightweight check: returns `true` if symphonia can probe `path` as audio.
/// Opens and probes the file but does not decode any audio frames.
pub fn probe_audio(path: &Path) -> bool {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn mono_is_duplicated_to_stereo() {
        let stereo = crate::decoder::normalize_to_stereo(vec![vec![0.25, -0.5]]).unwrap();
        assert_eq!(stereo, vec![vec![0.25, -0.5], vec![0.25, -0.5]]);
    }

    #[test]
    fn multichannel_is_rejected() {
        let err =
            crate::decoder::normalize_to_stereo(vec![vec![1.0], vec![0.0], vec![0.5]]).unwrap_err();
        assert!(err
            .to_string()
            .contains("only mono and stereo are supported"));
    }

    #[test]
    fn s16be_output_is_interleaved_and_big_endian() {
        let bytes = crate::output::stereo_to_s16be(&[vec![1.0, -1.0], vec![0.0, 0.5]]);
        assert_eq!(bytes.len(), 8);
        assert_eq!(&bytes[0..2], &i16::MAX.to_be_bytes());
        assert_eq!(&bytes[2..4], &0i16.to_be_bytes());
        assert_eq!(&bytes[4..6], &i16::MIN.to_be_bytes());
        assert_eq!(&bytes[6..8], &16384i16.to_be_bytes());
    }

    #[test]
    fn wav_output_is_stereo_44100_s16() {
        let wav = crate::output::stereo_to_wav(&[vec![0.0, 0.5], vec![-0.5, 1.0]]).unwrap();
        let reader = hound::WavReader::new(std::io::Cursor::new(wav)).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.channels, 2);
        assert_eq!(spec.sample_rate, crate::decoder::TARGET_SAMPLE_RATE);
        assert_eq!(spec.bits_per_sample, 16);
        assert_eq!(spec.sample_format, hound::SampleFormat::Int);
        assert_eq!(reader.duration(), 2);
    }

    #[test]
    fn streaming_resampler_matches_expected_length() {
        // Multi-chunk input (well over RESAMPLE_CHUNK_FRAMES) at 48k -> 44.1k.
        let frames = 48_000usize; // 1 second
        let left: Vec<f32> = (0..frames).map(|i| (i as f32 * 0.001).sin()).collect();
        let right = left.clone();
        let out = crate::resample::resample(vec![left, right], 48_000, 44_100).unwrap();
        let expected = (44_100.0f64 / 48_000.0 * frames as f64).ceil() as usize;
        assert_eq!(out[0].len(), expected);
        assert_eq!(out[1].len(), expected);
    }

    #[test]
    fn decode_to_writer_streams_without_buffering() {
        // The writer-based API must produce identical bytes to the Vec API.
        let path = temp_wav_path();
        write_test_wav(&path, 48_000, 4800).unwrap();

        let buffered = decode_to_s16be_44100_stereo(&path).unwrap();
        let mut streamed = Vec::new();
        decode_to_s16be_44100_stereo_writer(&path, &mut streamed).unwrap();
        assert_eq!(buffered, streamed);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn decodes_wav_to_public_outputs() {
        let path = temp_wav_path();
        write_test_wav(&path, 48_000, 480).unwrap();

        let raw = decode_to_s16be_44100_stereo(&path).unwrap();
        assert_eq!(raw.len(), 441 * crate::decoder::TARGET_CHANNELS * 2);

        let wav = decode_to_wav_44100_stereo(&path).unwrap();
        let reader = hound::WavReader::new(std::io::Cursor::new(wav)).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.channels, 2);
        assert_eq!(spec.sample_rate, crate::decoder::TARGET_SAMPLE_RATE);
        assert_eq!(reader.duration(), 441);

        let _ = std::fs::remove_file(path);
    }

    fn temp_wav_path() -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("md_pcm_test_{nanos}.wav"))
    }

    fn write_test_wav(
        path: &std::path::Path,
        sample_rate: u32,
        frames: usize,
    ) -> std::result::Result<(), hound::Error> {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec)?;
        for frame in 0..frames {
            let sample = if frame % 2 == 0 { i16::MAX / 2 } else { 0 };
            writer.write_sample(sample)?;
        }
        writer.finalize()?;
        Ok(())
    }
}
