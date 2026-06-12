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
}
