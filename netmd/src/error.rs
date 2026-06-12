use std::fmt;

/// Errors corresponding to NetMD protocol reply status codes and transport failures.
#[derive(Debug)]
pub enum NetMDError {
    /// Status `0x08` — command not implemented by the device.
    NotImplemented,
    /// Status `0x0a` — command rejected by the device. Holds the raw reply hex.
    Rejected(String),
    /// An unexpected status byte was returned.
    UnknownStatus(u8),
    /// Device kept returning interim (`0x0f`) after the maximum number of attempts.
    MaxInterimAttempts,
    /// Underlying USB transport error.
    Usb(rusb::Error),
}

impl fmt::Display for NetMDError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NetMDError::NotImplemented => write!(f, "NetMD: command not implemented"),
            NetMDError::Rejected(hex) => write!(f, "NetMD: command rejected - {hex}"),
            NetMDError::UnknownStatus(s) => write!(f, "NetMD: unknown return status: 0x{s:02x}"),
            NetMDError::MaxInterimAttempts => {
                write!(f, "NetMD: max read attempts for interim status reached")
            }
            NetMDError::Usb(e) => write!(f, "NetMD: USB error: {e}"),
        }
    }
}

impl std::error::Error for NetMDError {}

impl From<rusb::Error> for NetMDError {
    fn from(value: rusb::Error) -> Self {
        NetMDError::Usb(value)
    }
}
