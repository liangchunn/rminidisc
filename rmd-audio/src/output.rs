use std::io::Cursor;

use crate::decoder::{TARGET_CHANNELS, TARGET_SAMPLE_RATE};
use crate::error::{Error, Result};

pub(crate) fn stereo_to_s16be(channels: &[Vec<f32>]) -> Vec<u8> {
    let frames = channels[0].len().min(channels[1].len());
    let mut out = Vec::with_capacity(frames * TARGET_CHANNELS * 2);
    for frame in 0..frames {
        for channel in channels.iter().take(TARGET_CHANNELS) {
            out.extend_from_slice(&f32_to_i16(channel[frame]).to_be_bytes());
        }
    }
    out
}

pub(crate) fn stereo_to_wav(channels: &[Vec<f32>]) -> Result<Vec<u8>> {
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
