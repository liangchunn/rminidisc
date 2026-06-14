use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    calculate_cutoff, Async, FixedAsync, Resampler, SincInterpolationParameters,
    SincInterpolationType, WindowFunction,
};

use crate::decoder::{RESAMPLE_CHUNK_FRAMES, TARGET_CHANNELS};
use crate::error::{Error, Result};

pub(crate) fn resample(
    channels: Vec<Vec<f32>>,
    source_rate: u32,
    target_rate: u32,
) -> Result<Vec<Vec<f32>>> {
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
