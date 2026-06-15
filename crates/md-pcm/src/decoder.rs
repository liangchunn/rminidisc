use std::fs::File;
use std::io::ErrorKind;
use std::path::Path;

use symphonia::core::audio::GenericAudioBufferRef;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

use crate::error::{Error, Result};
use crate::output::StereoSink;
use crate::resample::StreamResampler;

pub(crate) const TARGET_SAMPLE_RATE: u32 = 44_100;
pub(crate) const TARGET_CHANNELS: usize = 2;
pub(crate) const RESAMPLE_CHUNK_FRAMES: usize = 4096;

/// Decodes `path` and streams normalized 44.1 kHz stereo frames to `sink`.
///
/// The decode → (optional) resample → output pipeline runs incrementally,
/// one decoded packet at a time, so peak memory is bounded by the packet and
/// resampler chunk sizes rather than the full track length.
pub(crate) fn decode_to_44100_stereo_streaming(
    path: &Path,
    sink: &mut dyn StereoSink,
) -> Result<()> {
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

    // Require a declared sample rate up front (matches the old behaviour) even
    // though the authoritative rate comes from the decoded packet spec.
    let _ = codec_params.sample_rate.ok_or(Error::MissingSampleRate)?;

    // The resampler is created lazily on the first decoded packet, once the
    // true source rate and channel count are known.
    let mut pipeline: Option<Pipeline> = None;
    // Reusable planar scratch (one Vec per channel) and interleaved buffer.
    let mut planar: Vec<Vec<f32>> = Vec::new();
    let mut interleaved: Vec<f32> = Vec::new();
    let mut produced_any = false;

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

        if decoded.frames() == 0 {
            continue;
        }

        let spec = decoded.spec();
        let channel_count = spec.channels().count();
        if channel_count == 0 {
            return Err(Error::NoChannels);
        }
        let rate = spec.rate();

        let pipeline = match pipeline.as_mut() {
            Some(p) => {
                if p.channel_count() != channel_count || p.source_rate() != rate {
                    return Err(Error::ChannelCountChanged);
                }
                p
            }
            None => pipeline.insert(Pipeline::new(channel_count, rate)?),
        };

        // Decode into planar f32, normalize to interleaved stereo, push.
        decode_into_planar(&decoded, channel_count, &mut planar)?;
        normalize_interleave(&planar, channel_count, &mut interleaved)?;
        if interleaved.is_empty() {
            continue;
        }
        produced_any = true;
        pipeline.push(&interleaved, sink)?;
    }

    match pipeline {
        Some(p) if produced_any => p.finish(sink),
        _ => Err(Error::NoDecodedSamples),
    }
}

/// Either passes interleaved stereo straight through, or runs it through a
/// streaming resampler, depending on whether the source matches the target
/// rate.
enum Pipeline {
    Passthrough {
        channel_count: usize,
        source_rate: u32,
    },
    Resample {
        channel_count: usize,
        source_rate: u32,
        resampler: Box<StreamResampler>,
    },
}

impl Pipeline {
    fn new(channel_count: usize, source_rate: u32) -> Result<Self> {
        if source_rate == TARGET_SAMPLE_RATE {
            Ok(Pipeline::Passthrough {
                channel_count,
                source_rate,
            })
        } else {
            Ok(Pipeline::Resample {
                channel_count,
                source_rate,
                resampler: Box::new(StreamResampler::new(source_rate, TARGET_SAMPLE_RATE)?),
            })
        }
    }

    fn push(&mut self, interleaved: &[f32], sink: &mut dyn StereoSink) -> Result<()> {
        match self {
            Pipeline::Passthrough { .. } => sink.write_interleaved(interleaved),
            Pipeline::Resample { resampler, .. } => resampler.push(interleaved, sink),
        }
    }

    fn finish(self, sink: &mut dyn StereoSink) -> Result<()> {
        match self {
            Pipeline::Passthrough { .. } => sink.finish(),
            Pipeline::Resample { resampler, .. } => resampler.finish(sink),
        }
    }
}

impl Pipeline {
    fn channel_count(&self) -> usize {
        match self {
            Pipeline::Passthrough { channel_count, .. }
            | Pipeline::Resample { channel_count, .. } => *channel_count,
        }
    }

    fn source_rate(&self) -> u32 {
        match self {
            Pipeline::Passthrough { source_rate, .. }
            | Pipeline::Resample { source_rate, .. } => *source_rate,
        }
    }
}

fn decode_into_planar(
    decoded: &GenericAudioBufferRef<'_>,
    channel_count: usize,
    planar: &mut Vec<Vec<f32>>,
) -> Result<()> {
    if decoded.spec().channels().count() != channel_count {
        return Err(Error::BufferChannelCountChanged);
    }
    planar.clear();
    decoded.copy_to_vecs_planar::<f32>(planar);
    Ok(())
}

/// Converts planar channels to interleaved stereo (mono is duplicated). Rejects
/// anything other than mono/stereo.
fn normalize_interleave(
    planar: &[Vec<f32>],
    channel_count: usize,
    interleaved: &mut Vec<f32>,
) -> Result<()> {
    interleaved.clear();
    match channel_count {
        0 => return Err(Error::NoChannels),
        1 => {
            let mono = &planar[0];
            interleaved.reserve(mono.len() * TARGET_CHANNELS);
            for &s in mono {
                interleaved.push(s);
                interleaved.push(s);
            }
        }
        2 => {
            let frames = planar[0].len().min(planar[1].len());
            interleaved.reserve(frames * TARGET_CHANNELS);
            for (&l, &r) in planar[0].iter().zip(planar[1].iter()).take(frames) {
                interleaved.push(l);
                interleaved.push(r);
            }
        }
        count => return Err(Error::UnsupportedChannelCount(count)),
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn normalize_to_stereo(channels: Vec<Vec<f32>>) -> Result<Vec<Vec<f32>>> {
    match channels.len() {
        0 => Err(Error::NoChannels),
        1 => Ok(vec![channels[0].clone(), channels[0].clone()]),
        2 => Ok(channels),
        count => Err(Error::UnsupportedChannelCount(count)),
    }
}
