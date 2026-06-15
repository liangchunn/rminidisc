//! Crate error type and `Result` alias.
//!
//! [`NetMDError`] (re-exported at the crate root as `Error`) covers USB
//! transport failures, protocol rejections/unknown status bytes, parse errors,
//! and crypto failures. All fallible APIs return [`Result`].

use std::{array::TryFromSliceError, convert::Infallible};

pub type Result<T> = std::result::Result<T, NetMDError>;

/// Errors corresponding to NetMD protocol reply status codes and transport failures.
#[derive(Debug, thiserror::Error)]
pub enum NetMDError {
    /// Status `0x08` — command not implemented by the device.
    #[error("NetMD: command not implemented")]
    NotImplemented,
    /// Status `0x0a` — command rejected by the device. Holds the raw reply hex.
    #[error("NetMD: command rejected - {0}")]
    Rejected(String),
    /// An unexpected status byte was returned.
    #[error("NetMD: unknown return status: 0x{0:02x}")]
    UnknownStatus(u8),
    /// Device kept returning interim (`0x0f`) after the maximum number of attempts.
    #[error("NetMD: max read attempts for interim status reached")]
    MaxInterimAttempts,
    /// Underlying USB transport error.
    #[error("NetMD: USB error: {0}")]
    Usb(#[from] rusb::Error),
    /// Underlying USB transport error with operation context.
    #[error("NetMD: USB error while {context}: {source}")]
    UsbContext {
        context: String,
        #[source]
        source: rusb::Error,
    },
    /// No supported NetMD device was found.
    #[error("NetMD: cannot find supported NetMD device")]
    DeviceNotFound,
    /// The requested supported NetMD device was not found.
    #[error("NetMD: cannot find supported device {vendor_id:04x}:{product_id:04x}")]
    DeviceNotFoundById { vendor_id: u16, product_id: u16 },
    /// More than one supported NetMD device was found and no selector was given.
    #[error(
        "NetMD: multiple supported NetMD devices connected; specify one with --device <vid:pid>:\n{devices}"
    )]
    MultipleSupportedDevices { devices: String },
    /// Query construction failed.
    #[error("NetMD: invalid query: {0}")]
    InvalidQuery(String),
    /// Reply scanning/template matching failed.
    #[error("NetMD: scan failed: {0}")]
    Scan(String),
    /// The device returned data that matched the transport layer but not the expected shape.
    #[error("NetMD: unexpected response: {0}")]
    UnexpectedResponse(String),
    /// WAV/RIFF parsing failed.
    #[error("NetMD: invalid WAV data: {0}")]
    InvalidWav(String),
    /// Shift-JIS text conversion failed.
    #[error("NetMD: text encoding error: {0}")]
    TextEncoding(String),
    /// Caller-provided data failed validation.
    #[error("NetMD: invalid input: {0}")]
    InvalidInput(String),
    /// A fixed-size integer parser received a slice of the wrong length.
    #[error("NetMD: invalid data length: {0}")]
    InvalidDataLength(#[from] TryFromSliceError),
    /// A bulk write returned success without advancing.
    #[error("NetMD: bulk write made no progress")]
    BulkWriteNoProgress,
}

impl From<Infallible> for NetMDError {
    fn from(value: Infallible) -> Self {
        match value {}
    }
}
