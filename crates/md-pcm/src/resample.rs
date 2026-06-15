use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    calculate_cutoff, Async, FixedAsync, Indexing, Resampler, SincInterpolationParameters,
    SincInterpolationType, WindowFunction,
};

use crate::decoder::{RESAMPLE_CHUNK_FRAMES, TARGET_CHANNELS};
use crate::error::{Error, Result};
use crate::output::StereoSink;

fn make_resampler(ratio: f64) -> Result<Async<f32>> {
    let params = SincInterpolationParameters {
        sinc_len: 128,
        f_cutoff: calculate_cutoff(128, WindowFunction::Blackman2),
        interpolation: SincInterpolationType::Quadratic,
        oversampling_factor: 256,
        window: WindowFunction::Blackman2,
    };
    Async::<f32>::new_sinc(
        ratio,
        1.1,
        &params,
        RESAMPLE_CHUNK_FRAMES,
        TARGET_CHANNELS,
        FixedAsync::Input,
    )
    .map_err(Error::CreateResampler)
}

/// Streaming stereo sample-rate converter.
///
/// Frames are pushed in via [`StreamResampler::push`] in arbitrary-sized
/// interleaved chunks; converted output is written to the provided
/// [`StereoSink`] as soon as it is available. Memory use is bounded by the
/// resampler's fixed chunk size, independent of track length.
///
/// The output matches [`rubato::Resampler::process_all_into_buffer`]: the
/// leading delay frames are trimmed and the total output length is
/// `ceil(ratio * total_input_frames)`.
pub(crate) struct StreamResampler {
    resampler: Async<f32>,
    ratio: f64,
    /// Interleaved input frames awaiting a full processing chunk.
    pending: Vec<f32>,
    /// Reusable interleaved output scratch buffer.
    output: Vec<f32>,
    /// Leading output frames still to be discarded (resampler delay).
    frames_to_trim: usize,
    /// Total input frames pushed so far.
    total_input_frames: usize,
    /// Total output frames already emitted to the sink (post-trim).
    emitted_output_frames: usize,
}

impl StreamResampler {
    pub(crate) fn new(source_rate: u32, target_rate: u32) -> Result<Self> {
        if source_rate == 0 {
            return Err(Error::ZeroSampleRate);
        }
        let ratio = f64::from(target_rate) / f64::from(source_rate);
        let resampler = make_resampler(ratio)?;
        let frames_to_trim = resampler.output_delay();
        let output_cap = resampler.output_frames_max() * TARGET_CHANNELS;
        Ok(Self {
            resampler,
            ratio,
            pending: Vec::new(),
            output: vec![0.0; output_cap],
            frames_to_trim,
            total_input_frames: 0,
            emitted_output_frames: 0,
        })
    }

    /// Pushes a chunk of interleaved stereo frames. `interleaved.len()` must be
    /// even.
    pub(crate) fn push(&mut self, interleaved: &[f32], sink: &mut dyn StereoSink) -> Result<()> {
        debug_assert_eq!(interleaved.len() % TARGET_CHANNELS, 0);
        self.total_input_frames += interleaved.len() / TARGET_CHANNELS;
        self.pending.extend_from_slice(interleaved);
        self.process_full_chunks(sink)
    }

    /// Processes as many full input chunks as `pending` allows.
    fn process_full_chunks(&mut self, sink: &mut dyn StereoSink) -> Result<()> {
        loop {
            let need = self.resampler.input_frames_next();
            if self.pending.len() / TARGET_CHANNELS < need {
                return Ok(());
            }
            let nbr_in = self.run_chunk(need, None, sink)?;
            self.pending.drain(0..nbr_in * TARGET_CHANNELS);
        }
    }

    /// Runs one `process_into_buffer` call over the front of `pending`, emitting
    /// the (delay-trimmed) output to the sink. Returns input frames consumed.
    fn run_chunk(
        &mut self,
        available: usize,
        partial_len: Option<usize>,
        sink: &mut dyn StereoSink,
    ) -> Result<usize> {
        let input_adapter = InterleavedSlice::new(&self.pending, TARGET_CHANNELS, available)
            .map_err(Error::ResamplerInputBuffer)?;
        let out_frames = self.output.len() / TARGET_CHANNELS;
        let mut output_adapter =
            InterleavedSlice::new_mut(&mut self.output, TARGET_CHANNELS, out_frames)
                .map_err(Error::ResamplerOutputBuffer)?;
        let indexing = Indexing {
            input_offset: 0,
            output_offset: 0,
            active_channels_mask: None,
            partial_len,
        };
        let (nbr_in, nbr_out) = self
            .resampler
            .process_into_buffer(&input_adapter, &mut output_adapter, Some(&indexing))
            .map_err(Error::Resample)?;

        // Trim leading delay frames, then emit the remainder.
        let trim = self.frames_to_trim.min(nbr_out);
        self.frames_to_trim -= trim;
        let start = trim * TARGET_CHANNELS;
        let end = nbr_out * TARGET_CHANNELS;
        if end > start {
            self.emit(start, end, sink)?;
        }
        Ok(nbr_in)
    }

    /// Emits `self.output[start..end]`, capping total output at the expected
    /// length `ceil(ratio * total_input_frames)`.
    fn emit(&mut self, start: usize, end: usize, sink: &mut dyn StereoSink) -> Result<()> {
        let expected = self.expected_output_frames();
        let available_frames = (end - start) / TARGET_CHANNELS;
        let remaining = expected.saturating_sub(self.emitted_output_frames);
        let take = available_frames.min(remaining);
        if take == 0 {
            return Ok(());
        }
        let slice = &self.output[start..start + take * TARGET_CHANNELS];
        sink.write_interleaved(slice)?;
        self.emitted_output_frames += take;
        Ok(())
    }

    fn expected_output_frames(&self) -> usize {
        (self.ratio * self.total_input_frames as f64).ceil() as usize
    }

    /// Flushes the remaining buffered input and the resampler's internal state,
    /// then finalizes the sink. Mirrors the tail of
    /// `process_all_into_buffer`: a final partial chunk followed by zero-pumping
    /// until the expected output length is reached.
    pub(crate) fn finish(mut self, sink: &mut dyn StereoSink) -> Result<()> {
        let expected = self.expected_output_frames();

        // Final partial chunk with whatever frames remain. The input adapter
        // must expose a full chunk's worth of frames, so pad with silence and
        // signal the real count via `partial_len`.
        let leftover = self.pending.len() / TARGET_CHANNELS;
        if leftover > 0 {
            let need = self.resampler.input_frames_next();
            self.pending.resize(need * TARGET_CHANNELS, 0.0);
            self.run_chunk(need, Some(leftover), sink)?;
            self.pending.clear();
        }

        // Zero-pump until we have produced the expected number of frames
        // (the resampler still holds buffered samples behind its delay line).
        let need = self.resampler.input_frames_next();
        self.pending.clear();
        self.pending.resize(need * TARGET_CHANNELS, 0.0);
        let mut guard = 0usize;
        while self.emitted_output_frames < expected {
            self.run_chunk(need, Some(0), sink)?;
            guard += 1;
            // Safety valve: each pump emits ~output_frames_max frames, so the
            // loop terminates quickly; bound it regardless.
            if guard > expected + 16 {
                break;
            }
        }

        sink.finish()
    }
}

/// Fully-buffered convenience wrapper used by tests. Resamples planar stereo to
/// the target rate and returns planar stereo.
#[cfg(test)]
pub(crate) fn resample(
    channels: Vec<Vec<f32>>,
    source_rate: u32,
    target_rate: u32,
) -> Result<Vec<Vec<f32>>> {
    use crate::output::CollectSink;

    if channels.len() != TARGET_CHANNELS {
        return Err(Error::ResampleNotStereo(channels.len()));
    }
    let frames = channels[0].len().min(channels[1].len());
    let mut interleaved = Vec::with_capacity(frames * TARGET_CHANNELS);
    for i in 0..frames {
        interleaved.push(channels[0][i]);
        interleaved.push(channels[1][i]);
    }
    let mut sink = CollectSink::default();
    let mut rs = StreamResampler::new(source_rate, target_rate)?;
    rs.push(&interleaved, &mut sink)?;
    rs.finish(&mut sink)?;
    Ok(sink.into_planar_stereo())
}
