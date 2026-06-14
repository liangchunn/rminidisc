//! Playback / transport control commands.
//!
//! Ports the playback section of `netmd-interface.ts:305-424` plus
//! `getRecordingParameters` (`:700`). None of these require crypto or bulk
//! transfers — they are plain control-transfer commands.
//!
//! HARDWARE NOTE: these commands are code-complete and mirror the JS reference
//! 1:1 (scan templates included), but the transport/seek/eject paths have not
//! been exercised against real hardware in this repository.

use log::{debug, trace};
use rusb::{DeviceHandle, UsbContext};

use crate::{
    descriptor::{change_descriptor_state, Descriptor, DescriptorAction},
    error::{NetMDError, Result},
    transport::{send_query, send_query_ext},
    util::{parse_bcd_u8, parse_u16},
};

/// Transport actions for `_play` (`netmd-interface.ts:27`).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Play = 0x75,
    Pause = 0x7d,
    FastForward = 0x39,
    Rewind = 0x49,
}

/// Track-change directions for `_trackChange` (`netmd-interface.ts:34`).
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrackChange {
    Previous = 0x0002,
    Next = 0x8001,
    Restart = 0x0001,
}

/// Issues a transport action. Mirrors `NetMDInterface._play` (`:361`).
fn play_action<T: UsbContext>(handle: &DeviceHandle<T>, action: Action) -> Result<()> {
    let query = format!("00 18c3 ff {:02x} 000000", action as u8);
    let reply = send_query(handle, query)?;
    reply.scan("%? 18c3 00 %b 000000")?;
    Ok(())
}

/// Starts playback. Mirrors `NetMDInterface.play`.
pub fn play<T: UsbContext>(handle: &DeviceHandle<T>) -> Result<()> {
    debug!("play");
    play_action(handle, Action::Play)
}

/// Pauses playback. Mirrors `NetMDInterface.pause`.
pub fn pause<T: UsbContext>(handle: &DeviceHandle<T>) -> Result<()> {
    debug!("pause");
    play_action(handle, Action::Pause)
}

/// Fast-forwards. Mirrors `NetMDInterface.fast_forward`.
pub fn fast_forward<T: UsbContext>(handle: &DeviceHandle<T>) -> Result<()> {
    debug!("fast forward");
    play_action(handle, Action::FastForward)
}

/// Rewinds. Mirrors `NetMDInterface.rewind`.
pub fn rewind<T: UsbContext>(handle: &DeviceHandle<T>) -> Result<()> {
    debug!("rewind");
    play_action(handle, Action::Rewind)
}

/// Stops playback. Mirrors `NetMDInterface.stop` (`:383`).
///
/// As in the JS reference, errors are swallowed (a fix for the Sony LAM-1).
pub fn stop<T: UsbContext>(handle: &DeviceHandle<T>) -> Result<()> {
    debug!("stop");
    match send_query(handle, "00 18c5 ff 00000000") {
        Ok(reply) => {
            let _ = reply.scan("%? 18c5 00 00000000");
        }
        Err(e) => trace!("stop ignored error: {e}"),
    }
    Ok(())
}

/// Seeks to the start of `track` (0-based). Mirrors `NetMDInterface.gotoTrack`
/// (`:393`). Returns the resulting track index.
pub fn goto_track<T: UsbContext>(handle: &DeviceHandle<T>, track: u16) -> Result<u16> {
    debug!("goto track #{track}");
    let query = format!("00 1850 ff010000 0000 {:04x}", track);
    let reply = send_query(handle, query)?;
    let data = reply.scan("%? 1850 00010000 0000 %w")?;
    Ok(parse_u16(data[0])?)
}

/// Seeks to a time within `track` (0-based). `time` is `[hour, minute, second,
/// frame]`. Mirrors `NetMDInterface.gotoTime` (`:400`).
pub fn goto_time<T: UsbContext>(handle: &DeviceHandle<T>, track: u16, time: [u8; 4]) -> Result<()> {
    debug!("goto time #{track} {time:?}");
    let query = format!(
        "00 1850 ff000000 0000 {:04x} {:02x}{:02x}{:02x}{:02x}",
        track, time[0], time[1], time[2], time[3]
    );
    let reply = send_query(handle, query)?;
    reply.scan("%? 1850 00000000 %?%? %w %B %B %B %B")?;
    Ok(())
}

/// Issues a track-change. Mirrors `NetMDInterface._trackChange` (`:408`).
fn track_change<T: UsbContext>(handle: &DeviceHandle<T>, direction: TrackChange) -> Result<()> {
    let query = format!("00 1850 ff10 00000000 {:04x}", direction as u16);
    let reply = send_query(handle, query)?;
    reply.scan("%? 1850 0010 00000000 %?%?")?;
    Ok(())
}

/// Skips to the next track. Mirrors `NetMDInterface.nextTrack`.
pub fn next_track<T: UsbContext>(handle: &DeviceHandle<T>) -> Result<()> {
    debug!("next track");
    track_change(handle, TrackChange::Next)
}

/// Skips to the previous track. Mirrors `NetMDInterface.previousTrack`.
pub fn previous_track<T: UsbContext>(handle: &DeviceHandle<T>) -> Result<()> {
    debug!("previous track");
    track_change(handle, TrackChange::Previous)
}

/// Restarts the current track. Mirrors `NetMDInterface.restartTrack`.
pub fn restart_track<T: UsbContext>(handle: &DeviceHandle<T>) -> Result<()> {
    debug!("restart track");
    track_change(handle, TrackChange::Restart)
}

/// Ejects the disc. Mirrors `NetMDInterface.ejectDisc` (`:347`).
pub fn eject_disc<T: UsbContext>(handle: &DeviceHandle<T>) -> Result<()> {
    debug!("eject disc");
    send_query(handle, "00 18c1 ff 6000")?;
    Ok(())
}

/// Returns true when the disc can be ejected. Mirrors `NetMDInterface.canEjectDisc`
/// (`:352`): the eject query is issued with interim acceptance and any error is
/// treated as "cannot eject".
pub fn can_eject_disc<T: UsbContext>(handle: &DeviceHandle<T>) -> bool {
    debug!("can eject disc?");
    send_query_ext(handle, "00 18c1 ff 6000", true).is_ok()
}

/// Reads a raw playback-status block. Mirrors `NetMDInterface._getPlaybackStatus`
/// (`:305`).
fn get_playback_status<T: UsbContext>(
    handle: &DeviceHandle<T>,
    p1: u16,
    p2: u16,
) -> Result<Vec<u8>> {
    change_descriptor_state(
        handle,
        Descriptor::OperatingStatusBlock,
        DescriptorAction::OpenRead,
    )?;
    let query = format!(
        "00 1809 8001 0330 {:04x} 0030 8805 0030 {:04x} 00 ff00 00000000",
        p1, p2
    );
    let reply = send_query(handle, query)?;
    // The JS reference template ends with a trailing `%?` to tolerate an extra
    // byte on some devices (MZ-RH1). `scanQuery` ignores trailing template
    // tokens, but this crate's `scan` requires the template to consume the whole
    // buffer, so we end at `%x` (the variable-length status block) and let it
    // absorb the remainder instead.
    let data = reply.scan("%? 1809 8001 0330 %?%? %?%? %?%? %?%? %?%? %? 1000 00%?0000 %x")?;
    let status = data[0].to_vec();
    change_descriptor_state(
        handle,
        Descriptor::OperatingStatusBlock,
        DescriptorAction::Close,
    )?;
    Ok(status)
}

/// Reads playback status block 1. Mirrors `NetMDInterface.getPlaybackStatus1`.
pub fn get_playback_status1<T: UsbContext>(handle: &DeviceHandle<T>) -> Result<Vec<u8>> {
    trace!("get playback status 1");
    get_playback_status(handle, 0x8801, 0x8807)
}

/// Reads playback status block 2. Mirrors `NetMDInterface.getPlaybackStatus2`.
pub fn get_playback_status2<T: UsbContext>(handle: &DeviceHandle<T>) -> Result<Vec<u8>> {
    trace!("get playback status 2");
    get_playback_status(handle, 0x8802, 0x8806)
}

/// Reads the current position as `[track, hour, minute, second, frame]`.
/// Mirrors `NetMDInterface.getPosition` (`:322`).
///
/// Returns `Ok(None)` when the device rejects the command (no current
/// position), matching the JS `null` return.
pub fn get_position<T: UsbContext>(handle: &DeviceHandle<T>) -> Result<Option<[u32; 5]>> {
    trace!("get position");
    change_descriptor_state(
        handle,
        Descriptor::OperatingStatusBlock,
        DescriptorAction::OpenRead,
    )?;
    let query = "00 1809 8001 0430 8802 0030 8805 0030 0003 0030 0002 00 ff00 00000000";
    let reply = match send_query(handle, query) {
        Ok(reply) => reply,
        Err(NetMDError::Rejected(_)) => {
            // A rejected command means "no position available" (JS returns null).
            let _ = change_descriptor_state(
                handle,
                Descriptor::OperatingStatusBlock,
                DescriptorAction::Close,
            );
            return Ok(None);
        }
        Err(e) => {
            return Err(e);
        }
    };
    let data = reply.scan(
        "%? 1809 8001 0430 %?%? %?%? %?%? %?%? %?%? %?%? %?%? %? %?00 00%?0000 000b 0002 0007 00 %w %B %B %B %B",
    )?;
    let position = [
        parse_u16(data[0])? as u32,
        parse_bcd_u8(data[1])? as u32,
        parse_bcd_u8(data[2])? as u32,
        parse_bcd_u8(data[3])? as u32,
        parse_bcd_u8(data[4])? as u32,
    ];
    change_descriptor_state(
        handle,
        Descriptor::OperatingStatusBlock,
        DescriptorAction::Close,
    )?;
    Ok(Some(position))
}

/// Reads `(channel_count_byte, encoding_byte)` recording parameters.
/// Mirrors `NetMDInterface.getRecordingParameters` (`:700`).
pub fn get_recording_parameters<T: UsbContext>(handle: &DeviceHandle<T>) -> Result<[u8; 2]> {
    trace!("get recording parameters");
    change_descriptor_state(
        handle,
        Descriptor::OperatingStatusBlock,
        DescriptorAction::OpenRead,
    )?;
    let query = "00 1809 8001 0330 8801 0030 8805 0030 8807 00 ff00 00000000";
    let reply = send_query(handle, query)?;
    let data = reply.scan(
        "%? 1809 8001 0330 8801 0030 8805 0030 8807 00 1000 000e0000 000c 8805 0008 80e0 0110 %b %b 4000",
    )?;
    let channels = crate::util::parse_u8(data[0])?;
    let encoding = crate::util::parse_u8(data[1])?;
    change_descriptor_state(
        handle,
        Descriptor::OperatingStatusBlock,
        DescriptorAction::Close,
    )?;
    Ok([channels, encoding])
}
