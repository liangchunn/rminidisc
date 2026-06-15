//! Playback and transport control.
//!
//! Start/stop/pause, fast-forward and rewind, track and time seeking
//! ([`NetMD::goto_track`], [`NetMD::goto_time`]), eject, and position/parameter
//! queries.

use log::{debug, trace};

use crate::{
    descriptor::{Descriptor, DescriptorAction},
    error::{NetMDError, Result},
    query::QueryBuilder,
    util::{parse_bcd_u8, parse_u16},
};

use super::NetMD;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Play = 0x75,
    Pause = 0x7d,
    FastForward = 0x39,
    Rewind = 0x49,
}

#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrackChange {
    Previous = 0x0002,
    Next = 0x8001,
    Restart = 0x0001,
}

impl NetMD {
    fn play_action(&self, action: Action) -> Result<()> {
        let query = QueryBuilder::new()
            .raw("00 18c3 ff")?
            .u8(action as u8)
            .raw("000000")?;
        let reply = self.send_query(query)?;
        reply.scan("%? 18c3 00 %b 000000")?;
        Ok(())
    }

    /// Starts playback. Mirrors `NetMDInterface.play`.
    pub fn play(&self) -> Result<()> {
        debug!("play");
        self.play_action(Action::Play)
    }

    /// Pauses playback. Mirrors `NetMDInterface.pause`.
    pub fn pause(&self) -> Result<()> {
        debug!("pause");
        self.play_action(Action::Pause)
    }

    /// Fast-forwards. Mirrors `NetMDInterface.fast_forward`.
    pub fn fast_forward(&self) -> Result<()> {
        debug!("fast forward");
        self.play_action(Action::FastForward)
    }

    /// Rewinds. Mirrors `NetMDInterface.rewind`.
    pub fn rewind(&self) -> Result<()> {
        debug!("rewind");
        self.play_action(Action::Rewind)
    }

    /// Stops playback. Mirrors `NetMDInterface.stop` (`:383`).
    ///
    /// As in the JS reference, errors are swallowed (a fix for the Sony LAM-1).
    pub fn stop(&self) -> Result<()> {
        debug!("stop");
        match self.send_query(QueryBuilder::new().raw("00 18c5 ff 00000000")?) {
            Ok(reply) => {
                let _ = reply.scan("%? 18c5 00 00000000");
            }
            Err(e) => trace!("stop ignored error: {e}"),
        }
        Ok(())
    }

    /// Seeks to the start of `track` (0-based). Mirrors `NetMDInterface.gotoTrack`
    /// (`:393`). Returns the resulting track index.
    pub fn goto_track(&self, track: u16) -> Result<u16> {
        debug!("goto track #{track}");
        let query = QueryBuilder::new().raw("00 1850 ff010000 0000")?.u16(track);
        let reply = self.send_query(query)?;
        let data = reply.scan("%? 1850 00010000 0000 %w")?;
        Ok(parse_u16(data[0])?)
    }

    /// Seeks to a time within `track` (0-based). `time` is `[hour, minute, second,
    /// frame]`. Mirrors `NetMDInterface.gotoTime` (`:400`).
    pub fn goto_time(&self, track: u16, time: [u8; 4]) -> Result<()> {
        debug!("goto time #{track} {time:?}");
        let query = QueryBuilder::new()
            .raw("00 1850 ff000000 0000")?
            .u16(track)
            .bytes(&time);
        let reply = self.send_query(query)?;
        reply.scan("%? 1850 00000000 %?%? %w %B %B %B %B")?;
        Ok(())
    }

    fn track_change(&self, direction: TrackChange) -> Result<()> {
        let query = QueryBuilder::new()
            .raw("00 1850 ff10 00000000")?
            .u16(direction as u16);
        let reply = self.send_query(query)?;
        reply.scan("%? 1850 0010 00000000 %?%?")?;
        Ok(())
    }

    /// Skips to the next track. Mirrors `NetMDInterface.nextTrack`.
    pub fn next_track(&self) -> Result<()> {
        debug!("next track");
        self.track_change(TrackChange::Next)
    }

    /// Skips to the previous track. Mirrors `NetMDInterface.previousTrack`.
    pub fn previous_track(&self) -> Result<()> {
        debug!("previous track");
        self.track_change(TrackChange::Previous)
    }

    /// Restarts the current track. Mirrors `NetMDInterface.restartTrack`.
    pub fn restart_track(&self) -> Result<()> {
        debug!("restart track");
        self.track_change(TrackChange::Restart)
    }

    /// Ejects the disc. Mirrors `NetMDInterface.ejectDisc` (`:347`).
    pub fn eject_disc(&self) -> Result<()> {
        debug!("eject disc");
        self.send_query(QueryBuilder::new().raw("00 18c1 ff 6000")?)?;
        Ok(())
    }

    /// Returns true when the disc can be ejected. Mirrors `NetMDInterface.canEjectDisc`
    /// (`:352`): the eject query is issued with interim acceptance and any error is
    /// treated as "cannot eject".
    pub fn can_eject_disc(&self) -> bool {
        debug!("can eject disc?");
        let Ok(query) = QueryBuilder::new().raw("00 18c1 ff 6000") else {
            return false;
        };
        self.send_query_ext(query, true).is_ok()
    }

    fn get_playback_status(&self, p1: u16, p2: u16) -> Result<Vec<u8>> {
        self.change_descriptor_state(Descriptor::OperatingStatusBlock, DescriptorAction::OpenRead)?;
        let query = QueryBuilder::new()
            .raw("00 1809 8001 0330")?
            .u16(p1)
            .raw("0030 8805 0030")?
            .u16(p2)
            .raw("00 ff00 00000000")?;
        let reply = self.send_query(query)?;
        let data = reply.scan("%? 1809 8001 0330 %?%? %?%? %?%? %?%? %?%? %? 1000 00%?0000 %x")?;
        let status = data[0].to_vec();
        self.change_descriptor_state(Descriptor::OperatingStatusBlock, DescriptorAction::Close)?;
        Ok(status)
    }

    /// Reads playback status block 1. Mirrors `NetMDInterface.getPlaybackStatus1`.
    pub fn get_playback_status1(&self) -> Result<Vec<u8>> {
        trace!("get playback status 1");
        self.get_playback_status(0x8801, 0x8807)
    }

    /// Reads playback status block 2. Mirrors `NetMDInterface.getPlaybackStatus2`.
    pub fn get_playback_status2(&self) -> Result<Vec<u8>> {
        trace!("get playback status 2");
        self.get_playback_status(0x8802, 0x8806)
    }

    /// Reads the current position as `[track, hour, minute, second, frame]`.
    /// Mirrors `NetMDInterface.getPosition` (`:322`).
    ///
    /// Returns `Ok(None)` when the device rejects the command (no current
    /// position), matching the JS `null` return.
    pub fn get_position(&self) -> Result<Option<[u32; 5]>> {
        trace!("get position");
        self.change_descriptor_state(Descriptor::OperatingStatusBlock, DescriptorAction::OpenRead)?;
        let query = "00 1809 8001 0430 8802 0030 8805 0030 0003 0030 0002 00 ff00 00000000";
        let reply = match self.send_query(query) {
            Ok(reply) => reply,
            Err(NetMDError::Rejected(_)) => {
                let _ = self.change_descriptor_state(
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
        self.change_descriptor_state(Descriptor::OperatingStatusBlock, DescriptorAction::Close)?;
        Ok(Some(position))
    }

    /// Reads `(channel_count_byte, encoding_byte)` recording parameters.
    /// Mirrors `NetMDInterface.getRecordingParameters` (`:700`).
    pub fn get_recording_parameters(&self) -> Result<[u8; 2]> {
        trace!("get recording parameters");
        self.change_descriptor_state(Descriptor::OperatingStatusBlock, DescriptorAction::OpenRead)?;
        let query = "00 1809 8001 0330 8801 0030 8805 0030 8807 00 ff00 00000000";
        let reply = self.send_query(query)?;
        let data = reply.scan(
            "%? 1809 8001 0330 8801 0030 8805 0030 8807 00 1000 000e0000 000c 8805 0008 80e0 0110 %b %b 4000",
        )?;
        let channels = crate::util::parse_u8(data[0])?;
        let encoding = crate::util::parse_u8(data[1])?;
        self.change_descriptor_state(Descriptor::OperatingStatusBlock, DescriptorAction::Close)?;
        Ok([channels, encoding])
    }
}
