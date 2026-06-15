//! Shared data types used across the crate's public API.
//!
//! Includes the structured listing types ([`Disc`], [`Group`], [`Track`]),
//! format/encoding enums ([`Wireformat`], [`Encoding`], [`DiscFormat`],
//! [`ChannelCount`]), status types ([`DeviceStatus`], [`OperatingStatus`],
//! [`PlaybackState`]).

use std::fmt::Display;

use crate::scan::scan;

pub(crate) const USB_TIMEOUT_MILLIS: u64 = 500;

pub(crate) const INTERIM_RETRY_INTERVAL_MS: u64 = 100;

pub(crate) const MAX_INTERIM_ATTEMPTS: u32 = 4;

pub(crate) const READ_REPLY_POLL_INTERVAL_MS: u64 = 10;

#[repr(u8)]
#[derive(Debug)]
pub(crate) enum ProtocolReply {
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
    /// Any status byte not defined by the protocol. Carries the raw byte so the
    /// transport layer can surface it as [`crate::error::NetMDError::UnknownStatus`]
    /// instead of panicking on unexpected hardware/bus data.
    #[allow(dead_code)] // raw byte retained for diagnostics even if unread
    Unknown(u8),
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
            other => ProtocolReply::Unknown(other),
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

impl TrackFlag {
    /// Interprets the raw track-flag byte from `getTrackFlags`. Mirrors the
    /// `flags as TrackFlag` cast in `listContent` (`netmd-commands.ts:222`):
    /// the protected bit pattern (`0x03`) means protected, anything else is
    /// treated as unprotected.
    #[must_use]
    pub const fn from_byte(value: u8) -> Self {
        if value == TrackFlag::Protected as u8 {
            TrackFlag::Protected
        } else {
            TrackFlag::Unprotected
        }
    }
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
    #[must_use]
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
    #[must_use]
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
    #[must_use]
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

pub(crate) const FRAME_SIZE: &[(Wireformat, usize)] = &[
    (Wireformat::Pcm, 2048),
    (Wireformat::Lp2, 192),
    (Wireformat::L105kbps, 152),
    (Wireformat::Lp4, 96),
];

pub(crate) struct ReadRequestHeader(pub(crate) [u8; 4]);

impl Default for ReadRequestHeader {
    fn default() -> Self {
        Self::new()
    }
}

impl ReadRequestHeader {
    pub(crate) fn new() -> Self {
        ReadRequestHeader([0; 4])
    }

    pub(crate) fn len(&self) -> usize {
        self.0[2] as usize
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A single track entry in a [`Disc`] listing. Mirrors the `Track` interface
/// (`netmd-commands.ts:81`). `index` is 0-based; titles are `None` when empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    pub index: u16,
    pub title: Option<String>,
    pub full_width_title: Option<String>,
    /// Duration in NetMD frames (see [`crate::util::time_to_frames`]).
    pub duration_frames: u32,
    pub channel: ChannelCount,
    pub encoding: Encoding,
    pub protected: TrackFlag,
}

/// A group of contiguous tracks. Mirrors the `Group` interface
/// (`netmd-commands.ts:91`). A `title` of `None` is the synthetic "ungrouped"
/// bucket holding tracks not assigned to any group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub index: usize,
    pub title: Option<String>,
    pub full_width_title: Option<String>,
    pub tracks: Vec<Track>,
}

/// A full disc content listing. Mirrors the `Disc` interface
/// (`netmd-commands.ts:98`). Capacity fields are in NetMD frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disc {
    pub title: String,
    pub full_width_title: String,
    pub writable: bool,
    pub write_protected: bool,
    pub used: u32,
    pub left: u32,
    pub total: u32,
    pub track_count: u8,
    pub groups: Vec<Group>,
}

#[derive(Debug)]
pub(crate) struct ReadRequestData(pub(crate) Vec<u8>);

impl ReadRequestData {
    pub(crate) fn scan<'b, 'a: 'b>(
        &'a self,
        template: &'a str,
    ) -> crate::error::Result<Vec<&'b [u8]>> {
        scan(template, &self.0)
    }
}

impl ReadRequestData {
    pub(crate) fn new(size: usize) -> Self {
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
