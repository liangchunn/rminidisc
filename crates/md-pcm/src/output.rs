use std::io::Write;

use crate::decoder::{TARGET_CHANNELS, TARGET_SAMPLE_RATE};
use crate::error::{Error, Result};

/// A streaming sink for interleaved stereo `f32` frames in `[-1.0, 1.0]`.
///
/// Implementations convert and write incrementally so the full track never has
/// to be materialized in memory.
pub(crate) trait StereoSink {
    /// Writes a chunk of interleaved stereo samples. `interleaved.len()` must be
    /// even (L,R pairs).
    fn write_interleaved(&mut self, interleaved: &[f32]) -> Result<()>;
    /// Flushes/finalizes the sink (e.g. patches WAV header sizes).
    fn finish(&mut self) -> Result<()>;
}

/// Writes interleaved stereo frames as signed 16-bit big-endian PCM.
pub(crate) struct S16beSink<W: Write> {
    writer: W,
    buf: Vec<u8>,
}

impl<W: Write> S16beSink<W> {
    pub(crate) fn new(writer: W) -> Self {
        Self {
            writer,
            buf: Vec::new(),
        }
    }
}

impl<W: Write> StereoSink for S16beSink<W> {
    fn write_interleaved(&mut self, interleaved: &[f32]) -> Result<()> {
        self.buf.clear();
        self.buf.reserve(interleaved.len() * 2);
        for &sample in interleaved {
            self.buf
                .extend_from_slice(&f32_to_i16(sample).to_be_bytes());
        }
        self.writer.write_all(&self.buf).map_err(Error::WriteOutput)
    }

    fn finish(&mut self) -> Result<()> {
        self.writer.flush().map_err(Error::WriteOutput)
    }
}

/// Writes interleaved stereo frames as a 44.1 kHz 16-bit PCM WAV file.
pub(crate) struct WavSink<W: Write + std::io::Seek> {
    writer: Option<hound::WavWriter<W>>,
}

impl<W: Write + std::io::Seek> WavSink<W> {
    pub(crate) fn new(writer: W) -> Result<Self> {
        let spec = hound::WavSpec {
            channels: TARGET_CHANNELS as u16,
            sample_rate: TARGET_SAMPLE_RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let wav = hound::WavWriter::new(writer, spec).map_err(Error::CreateWav)?;
        Ok(Self { writer: Some(wav) })
    }
}

impl<W: Write + std::io::Seek> StereoSink for WavSink<W> {
    fn write_interleaved(&mut self, interleaved: &[f32]) -> Result<()> {
        let wav = self.writer.as_mut().ok_or(Error::SinkAlreadyFinished)?;
        for &sample in interleaved {
            wav.write_sample(f32_to_i16(sample))
                .map_err(Error::WriteWavSample)?;
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        if let Some(wav) = self.writer.take() {
            wav.finalize().map_err(Error::FinalizeWav)?;
        }
        Ok(())
    }
}

/// Convenience: produce s16be bytes for an interleaved planar stereo buffer.
/// Retained for tests and callers that want a fully-buffered result.
#[cfg(test)]
pub(crate) fn stereo_to_s16be(channels: &[Vec<f32>]) -> Vec<u8> {
    let frames = channels[0].len().min(channels[1].len());
    let mut sink = S16beSink::new(Vec::with_capacity(frames * TARGET_CHANNELS * 2));
    let mut interleaved = vec![0.0f32; frames * TARGET_CHANNELS];
    for frame in 0..frames {
        for (ch, channel) in channels.iter().take(TARGET_CHANNELS).enumerate() {
            interleaved[frame * TARGET_CHANNELS + ch] = channel[frame];
        }
    }
    sink.write_interleaved(&interleaved).expect("vec write");
    sink.finish().expect("vec flush");
    sink.writer
}

/// Convenience: produce a WAV byte buffer for an interleaved planar stereo
/// buffer. Retained for tests and callers that want a fully-buffered result.
#[cfg(test)]
pub(crate) fn stereo_to_wav(channels: &[Vec<f32>]) -> Result<Vec<u8>> {
    let spec = hound::WavSpec {
        channels: TARGET_CHANNELS as u16,
        sample_rate: TARGET_SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = std::io::Cursor::new(Vec::new());
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

/// Test sink that accumulates interleaved frames for inspection.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct CollectSink {
    pub(crate) interleaved: Vec<f32>,
}

#[cfg(test)]
impl CollectSink {
    pub(crate) fn into_planar_stereo(self) -> Vec<Vec<f32>> {
        let frames = self.interleaved.len() / TARGET_CHANNELS;
        let mut left = Vec::with_capacity(frames);
        let mut right = Vec::with_capacity(frames);
        for frame in self.interleaved.chunks_exact(TARGET_CHANNELS) {
            left.push(frame[0]);
            right.push(frame[1]);
        }
        vec![left, right]
    }
}

#[cfg(test)]
impl StereoSink for CollectSink {
    fn write_interleaved(&mut self, interleaved: &[f32]) -> Result<()> {
        self.interleaved.extend_from_slice(interleaved);
        Ok(())
    }
    fn finish(&mut self) -> Result<()> {
        Ok(())
    }
}

fn f32_to_i16(sample: f32) -> i16 {
    let sample = sample.clamp(-1.0, 1.0);
    if sample <= -1.0 {
        i16::MIN
    } else {
        (sample * i16::MAX as f32).round() as i16
    }
}
