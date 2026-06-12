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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiscFlags {
    raw: u8,
}

impl DiscFlags {
    pub const fn from_bits(raw: u8) -> Self {
        Self { raw }
    }

    pub const fn raw(self) -> u8 {
        self.raw
    }

    pub const fn contains(self, flag: DiscFlag) -> bool {
        self.raw & flag as u8 != 0
    }

    pub const fn is_write_protected(self) -> bool {
        self.contains(DiscFlag::WriteProtected)
    }

    pub const fn is_writable(self) -> bool {
        self.contains(DiscFlag::Writable) && !self.is_write_protected()
    }

    pub const fn unknown_bits(self) -> u8 {
        self.raw & !(DiscFlag::Writable as u8 | DiscFlag::WriteProtected as u8)
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetMDLevel {
    Level1 = 0x20,
    Level2 = 0x50,
    Level3 = 0x70,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FullOperatingStatus {
    pub mode: u8,
    pub status: OperatingStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatingStatus {
    Ready,
    BlankDisc,
    Unknown(u16),
}

impl OperatingStatus {
    pub const fn raw(self) -> u16 {
        match self {
            OperatingStatus::Ready => 0xc5ff,
            OperatingStatus::BlankDisc => 0xffff,
            OperatingStatus::Unknown(value) => value,
        }
    }
}

/// High-level playback / operating state, mirroring the operating-status map in
/// `netmd-commands.ts:110`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Ready,
    Playing,
    Paused,
    FastForward,
    Rewind,
    ReadingToc,
    NoDisc,
    DiscBlank,
    ReadyForTransfer,
    Unknown(u16),
}

impl PlaybackState {
    /// Maps a raw 16-bit operating-status value to a [`PlaybackState`].
    /// Values mirror `netmd-commands.ts:110`.
    pub fn from_u16(value: u16) -> Self {
        match value {
            50687 => PlaybackState::Ready,
            50037 => PlaybackState::Playing,
            50045 => PlaybackState::Paused,
            49983 => PlaybackState::FastForward,
            49999 => PlaybackState::Rewind,
            65315 => PlaybackState::ReadingToc,
            65296 => PlaybackState::NoDisc,
            65535 => PlaybackState::DiscBlank,
            65319 => PlaybackState::ReadyForTransfer,
            other => PlaybackState::Unknown(other),
        }
    }
}

/// Position/time within the currently selected track. `minute` is the absolute
/// minute count (`hour * 60 + minute` from the device's `[track,h,m,s,f]`),
/// matching the derivation in `netmd-commands.ts:146`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaybackTime {
    pub minute: u32,
    pub second: u32,
    pub frame: u32,
}

/// A comprehensive playback status snapshot. Mirrors `DeviceStatus`
/// (`netmd-commands.ts:124`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceStatus {
    pub disc_present: bool,
    pub state: PlaybackState,
    pub track: Option<u32>,
    pub time: Option<PlaybackTime>,
}

pub const FRAME_SIZE: &[(Wireformat, usize)] = &[
    (Wireformat::Pcm, 2048),
    (Wireformat::Lp2, 192),
    (Wireformat::L105kbps, 152),
    (Wireformat::Lp4, 96),
];

pub struct ReadRequestHeader(pub [u8; 4]);

impl Default for ReadRequestHeader {
    fn default() -> Self {
        Self::new()
    }
}

impl ReadRequestHeader {
    pub fn new() -> Self {
        ReadRequestHeader([0; 4])
    }

    pub fn len(&self) -> usize {
        self.0[2] as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
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
