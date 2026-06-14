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

pub(crate) const TARGET_SAMPLE_RATE: u32 = 44_100;
pub(crate) const TARGET_CHANNELS: usize = 2;
pub(crate) const RESAMPLE_CHUNK_FRAMES: usize = 4096;

#[derive(Debug, Clone)]
pub(crate) struct DecodedAudio {
    pub(crate) sample_rate: u32,
    pub(crate) channels: Vec<Vec<f32>>,
}

#[derive(Debug, Clone)]
pub(crate) struct NormalizedAudio {
    pub(crate) channels: Vec<Vec<f32>>,
}

pub(crate) fn decode_to_44100_stereo(path: impl AsRef<Path>) -> Result<NormalizedAudio> {
    let path = path.as_ref();
    let decoded = decode_file(path)?;
    let stereo = normalize_to_stereo(decoded.channels)?;
    let channels = if decoded.sample_rate == TARGET_SAMPLE_RATE {
        stereo
    } else {
        crate::resample::resample(stereo, decoded.sample_rate, TARGET_SAMPLE_RATE)?
    };

    Ok(NormalizedAudio { channels })
}

pub(crate) fn decode_file(path: &Path) -> Result<DecodedAudio> {
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

pub(crate) fn normalize_to_stereo(channels: Vec<Vec<f32>>) -> Result<Vec<Vec<f32>>> {
    match channels.len() {
        0 => Err(Error::NoChannels),
        1 => Ok(vec![channels[0].clone(), channels[0].clone()]),
        2 => Ok(channels),
        count => Err(Error::UnsupportedChannelCount(count)),
    }
}
