use std::fmt::Display;

use crate::scan::scan;

pub const USB_TIMEOUT_MILLIS: u64 = 500;

pub const INTERIM_RETRY_INTERVAL_MS: u64 = 100;

pub const MAX_INTERIM_ATTEMPTS: u32 = 4;

pub const READ_REPLY_POLL_INTERVAL_MS: u64 = 10;

#[repr(u8)]
#[derive(Debug)]
pub enum ProtocolReply {
    Control = 0x00,
    Status = 0x01,
    SpecificInquiry = 0x02,
    Notify = 0x03,
    GeneralInquiry = 0x04,
    NotImplemented = 0x08,
    Accepted = 0x09,
    Rejected = 0x0a,
    InTransition = 0x0b,
    Implemented = 0x0c,
    Changed = 0x0d,
    Interim = 0x0f,
}

impl From<u8> for ProtocolReply {
    fn from(value: u8) -> Self {
        match value {
            0x00 => ProtocolReply::Control,
            0x01 => ProtocolReply::Status,
            0x02 => ProtocolReply::SpecificInquiry,
            0x03 => ProtocolReply::Notify,
            0x04 => ProtocolReply::GeneralInquiry,
            0x08 => ProtocolReply::NotImplemented,
            0x09 => ProtocolReply::Accepted,
            0x0a => ProtocolReply::Rejected,
            0x0b => ProtocolReply::InTransition,
            0x0c => ProtocolReply::Implemented,
            0x0d => ProtocolReply::Changed,
            0x0f => ProtocolReply::Interim,
            _ => unimplemented!(),
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscFormat {
    Lp4 = 0,
    Lp2 = 2,
    SpMono = 4,
    SpStereo = 6,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wireformat {
    Pcm = 0,
    L105kbps = 0x90,
    Lp2 = 0x94,
    Lp4 = 0xa8,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Sp = 0x90,
    Lp2 = 0x92,
    Lp4 = 0x93,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channels {
    Mono = 0x01,
    Stereo = 0x00,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelCount {
    Mono = 1,
    Stereo = 2,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackFlag {
    Protected = 0x03,
    Unprotected = 0x00,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscFlag {
    Writable = 0x10,
    WriteProtected = 0x40,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetMDLevel {
    Level1 = 0x20,
    Level2 = 0x50,
    Level3 = 0x70,
}

pub const FRAME_SIZE: &[(Wireformat, usize)] = &[
    (Wireformat::Pcm, 2048),
    (Wireformat::Lp2, 192),
    (Wireformat::L105kbps, 152),
    (Wireformat::Lp4, 96),
];

pub struct ReadRequestHeader(pub [u8; 4]);

impl ReadRequestHeader {
    pub fn new() -> Self {
        ReadRequestHeader([0; 4])
    }

    pub fn len(&self) -> usize {
        self.0[2] as usize
    }
}

#[derive(Debug)]
pub struct ReadRequestData(pub Vec<u8>);

impl ReadRequestData {
    pub fn scan<'b, 'a: 'b>(&'a self, template: &'a str) -> Result<Vec<&'b [u8]>, anyhow::Error> {
        scan(template, &self.0)
    }
}

impl ReadRequestData {
    pub fn new(size: usize) -> Self {
        ReadRequestData(vec![0; size])
    }
}

impl Display for ReadRequestData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[")?;
        for byte in &self.0 {
            write!(f, "0x{:02x}, ", byte)?;
        }
        write!(f, "]")?;
        Ok(())
    }
}
