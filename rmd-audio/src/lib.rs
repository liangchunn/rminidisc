//! Audio decoding and normalization used by the `rmd` upload command.
//!
//! This crate intentionally knows nothing about MiniDisc wire formats or ATRAC
//! encoding. It only replaces the old FFmpeg normalization steps:
//!
//! - `-ac 2 -ar 44100 -f s16be`
//! - `-ac 2 -ar 44100 -f wav`

use std::fs::File;
use std::io::{Cursor, ErrorKind};
use std::path::Path;

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::audioadapter_buffers::SizeError;
use rubato::{
    calculate_cutoff, Async, FixedAsync, ResampleError, Resampler, ResamplerConstructionError,
    SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use symphonia::core::audio::GenericAudioBufferRef;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

const TARGET_SAMPLE_RATE: u32 = 44_100;
const TARGET_CHANNELS: usize = 2;
const RESAMPLE_CHUNK_FRAMES: usize = 4096;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("opening audio file {path}: {source}")]
    OpenFile {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("probing audio format for {path}: {source}")]
    ProbeFormat {
        path: String,
        #[source]
        source: SymphoniaError,
    },
    #[error("no supported audio track")]
    NoSupportedAudioTrack,
    #[error("audio track has no codec parameters")]
    MissingCodecParameters,
    #[error("creating audio decoder: {0}")]
    CreateDecoder(#[source] SymphoniaError),
    #[error("audio track has no sample rate")]
    MissingSampleRate,
    #[error("reading audio packet: {0}")]
    ReadPacketIo(#[source] std::io::Error),
    #[error("reading audio packet: {0}")]
    ReadPacket(#[source] SymphoniaError),
    #[error("audio stream changed while decoding")]
    StreamChanged,
    #[error("decoding audio packet: {0}")]
    DecodePacketIo(#[source] std::io::Error),
    #[error("decoding audio packet: {0}")]
    DecodePacket(#[source] SymphoniaError),
    #[error("audio decoder reset required")]
    DecoderResetRequired,
    #[error("decoded audio has no channels")]
    NoChannels,
    #[error("audio channel count changed while decoding")]
    ChannelCountChanged,
    #[error("audio file produced no decoded samples")]
    NoDecodedSamples,
    #[error("decoded audio buffer channel count changed")]
    BufferChannelCountChanged,
    #[error("audio has {0} channels; only mono and stereo are supported")]
    UnsupportedChannelCount(usize),
    #[error("source sample rate is zero")]
    ZeroSampleRate,
    #[error("creating sample-rate converter: {0}")]
    CreateResampler(#[source] ResamplerConstructionError),
    #[error("creating resampler input buffer: {0}")]
    ResamplerInputBuffer(#[source] SizeError),
    #[error("creating resampler output buffer: {0}")]
    ResamplerOutputBuffer(#[source] SizeError),
    #[error("resampling audio: {0}")]
    Resample(#[source] ResampleError),
    #[error("creating WAV writer: {0}")]
    CreateWav(#[source] hound::Error),
    #[error("writing WAV sample: {0}")]
    WriteWavSample(#[source] hound::Error),
    #[error("finalizing WAV data: {0}")]
    FinalizeWav(#[source] hound::Error),
    #[error("resample requires stereo input, got {0} channel(s)")]
    ResampleNotStereo(usize),
}

#[derive(Debug, Clone)]
struct DecodedAudio {
    sample_rate: u32,
    channels: Vec<Vec<f32>>,
}

#[derive(Debug, Clone)]
struct NormalizedAudio {
    channels: Vec<Vec<f32>>,
}

/// Decode `path`, normalize it to stereo 44.1 kHz PCM, and return interleaved
/// signed 16-bit big-endian samples.
pub fn decode_to_s16be_44100_stereo(path: impl AsRef<Path>) -> Result<Vec<u8>> {
    let audio = decode_to_44100_stereo(path)?;
    Ok(stereo_to_s16be(&audio.channels))
}

/// Decode `path`, normalize it to stereo 44.1 kHz PCM, and return a WAV file
/// containing signed 16-bit little-endian PCM samples.
pub fn decode_to_wav_44100_stereo(path: impl AsRef<Path>) -> Result<Vec<u8>> {
    let audio = decode_to_44100_stereo(path)?;
    stereo_to_wav(&audio.channels)
}

fn decode_to_44100_stereo(path: impl AsRef<Path>) -> Result<NormalizedAudio> {
    let path = path.as_ref();
    let decoded = decode_file(path)?;
    let stereo = normalize_to_stereo(decoded.channels)?;
    let channels = if decoded.sample_rate == TARGET_SAMPLE_RATE {
        stereo
    } else {
        resample(stereo, decoded.sample_rate, TARGET_SAMPLE_RATE)?
    };

    Ok(NormalizedAudio { channels })
}

fn decode_file(path: &Path) -> Result<DecodedAudio> {
    let file = File::open(path).map_err(|source| Error::OpenFile {
        path: path.display().to_string(),
        source,
    })?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|ext| ext.to_str()) {
        hint.with_extension(ext);
    }

    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|source| Error::ProbeFormat {
            path: path.display().to_string(),
            source,
        })?;

    let track = format
        .default_track(TrackType::Audio)
        .ok_or(Error::NoSupportedAudioTrack)?;
    let track_id = track.id;
    let codec_params = track
        .codec_params
        .as_ref()
        .and_then(|params| params.audio())
        .ok_or(Error::MissingCodecParameters)?
        .clone();
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&codec_params, &AudioDecoderOptions::default())
        .map_err(Error::CreateDecoder)?;

    let mut sample_rate = codec_params.sample_rate.ok_or(Error::MissingSampleRate)?;
    let mut channels: Option<Vec<Vec<f32>>> = None;

    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(SymphoniaError::IoError(err)) if err.kind() == ErrorKind::UnexpectedEof => break,
            Err(SymphoniaError::IoError(err)) => return Err(Error::ReadPacketIo(err)),
            Err(SymphoniaError::ResetRequired) => return Err(Error::StreamChanged),
            Err(err) => return Err(Error::ReadPacket(err)),
        };

        if packet.track_id != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(SymphoniaError::IoError(err)) => return Err(Error::DecodePacketIo(err)),
            Err(SymphoniaError::ResetRequired) => return Err(Error::DecoderResetRequired),
            Err(err) => return Err(Error::DecodePacket(err)),
        };

        let spec = decoded.spec();
        sample_rate = spec.rate();
        let channel_count = spec.channels().count();
        if channel_count == 0 {
            return Err(Error::NoChannels);
        }

        let channels = channels.get_or_insert_with(|| vec![Vec::new(); channel_count]);
        if channels.len() != channel_count {
            return Err(Error::ChannelCountChanged);
        }

        append_f32_samples(&decoded, channels)?;
    }

    let channels = channels.ok_or(Error::NoDecodedSamples)?;
    if channels.iter().all(Vec::is_empty) {
        return Err(Error::NoDecodedSamples);
    }

    Ok(DecodedAudio {
        sample_rate,
        channels,
    })
}

fn append_f32_samples(
    decoded: &GenericAudioBufferRef<'_>,
    channels: &mut [Vec<f32>],
) -> Result<()> {
    if decoded.frames() == 0 {
        return Ok(());
    }
    if decoded.spec().channels().count() != channels.len() {
        return Err(Error::BufferChannelCountChanged);
    }

    let mut planar = Vec::new();
    decoded.copy_to_vecs_planar::<f32>(&mut planar);

    for (out, samples) in channels.iter_mut().zip(planar) {
        out.extend_from_slice(&samples);
    }

    Ok(())
}

fn normalize_to_stereo(channels: Vec<Vec<f32>>) -> Result<Vec<Vec<f32>>> {
    match channels.len() {
        0 => Err(Error::NoChannels),
        1 => Ok(vec![channels[0].clone(), channels[0].clone()]),
        2 => Ok(channels),
        count => Err(Error::UnsupportedChannelCount(count)),
    }
}

fn resample(channels: Vec<Vec<f32>>, source_rate: u32, target_rate: u32) -> Result<Vec<Vec<f32>>> {
    if source_rate == 0 {
        return Err(Error::ZeroSampleRate);
    }
    if channels.len() != TARGET_CHANNELS {
        return Err(Error::ResampleNotStereo(channels.len()));
    }

    let params = SincInterpolationParameters {
        sinc_len: 128,
        f_cutoff: calculate_cutoff(128, WindowFunction::Blackman2),
        interpolation: SincInterpolationType::Quadratic,
        oversampling_factor: 256,
        window: WindowFunction::Blackman2,
    };
    let ratio = f64::from(target_rate) / f64::from(source_rate);
    let mut resampler = Async::<f32>::new_sinc(
        ratio,
        1.1,
        &params,
        RESAMPLE_CHUNK_FRAMES,
        TARGET_CHANNELS,
        FixedAsync::Input,
    )
    .map_err(Error::CreateResampler)?;

    let input_frames = channels[0].len().min(channels[1].len());
    let input = interleave_stereo(&channels, input_frames);
    let input_adapter = InterleavedSlice::new(&input, TARGET_CHANNELS, input_frames)
        .map_err(Error::ResamplerInputBuffer)?;

    let output_frames = resampler.process_all_needed_output_len(input_frames);
    let mut output = vec![0.0f32; output_frames * TARGET_CHANNELS];
    let mut output_adapter = InterleavedSlice::new_mut(&mut output, TARGET_CHANNELS, output_frames)
        .map_err(Error::ResamplerOutputBuffer)?;

    let (_consumed, produced) = resampler
        .process_all_into_buffer(&input_adapter, &mut output_adapter, input_frames, None)
        .map_err(Error::Resample)?;

    output.truncate(produced * TARGET_CHANNELS);
    Ok(deinterleave_stereo(&output))
}

fn interleave_stereo(channels: &[Vec<f32>], frames: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(frames * TARGET_CHANNELS);
    for (&l, &r) in channels[0].iter().zip(channels[1].iter()).take(frames) {
        out.push(l);
        out.push(r);
    }
    out
}

fn deinterleave_stereo(samples: &[f32]) -> Vec<Vec<f32>> {
    let frames = samples.len() / TARGET_CHANNELS;
    let mut left = Vec::with_capacity(frames);
    let mut right = Vec::with_capacity(frames);
    for frame in samples.chunks_exact(TARGET_CHANNELS) {
        left.push(frame[0]);
        right.push(frame[1]);
    }
    vec![left, right]
}

fn stereo_to_s16be(channels: &[Vec<f32>]) -> Vec<u8> {
    let frames = channels[0].len().min(channels[1].len());
    let mut out = Vec::with_capacity(frames * TARGET_CHANNELS * 2);
    for frame in 0..frames {
        for channel in channels.iter().take(TARGET_CHANNELS) {
            out.extend_from_slice(&f32_to_i16(channel[frame]).to_be_bytes());
        }
    }
    out
}

fn stereo_to_wav(channels: &[Vec<f32>]) -> Result<Vec<u8>> {
    let spec = hound::WavSpec {
        channels: TARGET_CHANNELS as u16,
        sample_rate: TARGET_SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(&mut cursor, spec).map_err(Error::CreateWav)?;
        let frames = channels[0].len().min(channels[1].len());
        for frame in 0..frames {
            for channel in channels.iter().take(TARGET_CHANNELS) {
                writer
                    .write_sample(f32_to_i16(channel[frame]))
                    .map_err(Error::WriteWavSample)?;
            }
        }
        writer.finalize().map_err(Error::FinalizeWav)?;
    }
    Ok(cursor.into_inner())
}

fn f32_to_i16(sample: f32) -> i16 {
    let sample = sample.clamp(-1.0, 1.0);
    if sample <= -1.0 {
        i16::MIN
    } else {
        (sample * i16::MAX as f32).round() as i16
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn mono_is_duplicated_to_stereo() {
        let stereo = normalize_to_stereo(vec![vec![0.25, -0.5]]).unwrap();
        assert_eq!(stereo, vec![vec![0.25, -0.5], vec![0.25, -0.5]]);
    }

    #[test]
    fn multichannel_is_rejected() {
        let err = normalize_to_stereo(vec![vec![1.0], vec![0.0], vec![0.5]]).unwrap_err();
        assert!(err
            .to_string()
            .contains("only mono and stereo are supported"));
    }

    #[test]
    fn s16be_output_is_interleaved_and_big_endian() {
        let bytes = stereo_to_s16be(&[vec![1.0, -1.0], vec![0.0, 0.5]]);
        assert_eq!(bytes.len(), 8);
        assert_eq!(&bytes[0..2], &i16::MAX.to_be_bytes());
        assert_eq!(&bytes[2..4], &0i16.to_be_bytes());
        assert_eq!(&bytes[4..6], &i16::MIN.to_be_bytes());
        assert_eq!(&bytes[6..8], &16384i16.to_be_bytes());
    }

    #[test]
    fn wav_output_is_stereo_44100_s16() {
        let wav = stereo_to_wav(&[vec![0.0, 0.5], vec![-0.5, 1.0]]).unwrap();
        let reader = hound::WavReader::new(Cursor::new(wav)).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.channels, 2);
        assert_eq!(spec.sample_rate, TARGET_SAMPLE_RATE);
        assert_eq!(spec.bits_per_sample, 16);
        assert_eq!(spec.sample_format, hound::SampleFormat::Int);
        assert_eq!(reader.duration(), 2);
    }

    #[test]
    fn decodes_wav_to_public_outputs() {
        let path = temp_wav_path();
        write_test_wav(&path, 48_000, 480).unwrap();

        let raw = decode_to_s16be_44100_stereo(&path).unwrap();
        assert_eq!(raw.len(), 441 * TARGET_CHANNELS * 2);

        let wav = decode_to_wav_44100_stereo(&path).unwrap();
        let reader = hound::WavReader::new(Cursor::new(wav)).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.channels, 2);
        assert_eq!(spec.sample_rate, TARGET_SAMPLE_RATE);
        assert_eq!(reader.duration(), 441);

        let _ = std::fs::remove_file(path);
    }

    fn temp_wav_path() -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("rmd_audio_test_{nanos}.wav"))
    }

    fn write_test_wav(
        path: &Path,
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
