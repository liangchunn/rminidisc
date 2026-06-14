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
        source: symphonia::core::errors::Error,
    },
    #[error("no supported audio track")]
    NoSupportedAudioTrack,
    #[error("audio track has no codec parameters")]
    MissingCodecParameters,
    #[error("creating audio decoder: {0}")]
    CreateDecoder(#[source] symphonia::core::errors::Error),
    #[error("audio track has no sample rate")]
    MissingSampleRate,
    #[error("reading audio packet: {0}")]
    ReadPacketIo(#[source] std::io::Error),
    #[error("reading audio packet: {0}")]
    ReadPacket(#[source] symphonia::core::errors::Error),
    #[error("audio stream changed while decoding")]
    StreamChanged,
    #[error("decoding audio packet: {0}")]
    DecodePacketIo(#[source] std::io::Error),
    #[error("decoding audio packet: {0}")]
    DecodePacket(#[source] symphonia::core::errors::Error),
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
    CreateResampler(#[source] rubato::ResamplerConstructionError),
    #[error("creating resampler input buffer: {0}")]
    ResamplerInputBuffer(#[source] rubato::audioadapter_buffers::SizeError),
    #[error("creating resampler output buffer: {0}")]
    ResamplerOutputBuffer(#[source] rubato::audioadapter_buffers::SizeError),
    #[error("resampling audio: {0}")]
    Resample(#[source] rubato::ResampleError),
    #[error("creating WAV writer: {0}")]
    CreateWav(#[source] hound::Error),
    #[error("writing WAV sample: {0}")]
    WriteWavSample(#[source] hound::Error),
    #[error("finalizing WAV data: {0}")]
    FinalizeWav(#[source] hound::Error),
    #[error("resample requires stereo input, got {0} channel(s)")]
    ResampleNotStereo(usize),
}
