//! High-level aggregate commands that compose several lower-level calls.
//!
//! Provides [`NetMD::list_content`], which assembles a full structured [`Disc`]
//! (title, capacity, and every track grouped), and [`NetMD::get_device_status`],
//! which returns a [`DeviceStatus`] playback snapshot. Mirrors `netmd-commands.ts`.

use log::debug;

use crate::{
    error::Result,
    types::{DeviceStatus, Disc, Group, PlaybackState, PlaybackTime, Track, TrackFlag},
    util::time_to_frames,
};

use super::NetMD;

impl NetMD {
    /// Returns a comprehensive playback status snapshot. Mirrors
    /// `getDeviceStatus` (`netmd-commands.ts:131`).
    pub fn get_device_status(&self) -> Result<DeviceStatus> {
        debug!("get device status");
        let status = self.get_status()?;
        let playback_status2 = self.get_playback_status2()?;
        let position = self.get_position()?;

        let operating_status = playback_status2_to_operating_status(&playback_status2);
        let disc_present = status.get(4) != Some(&0x80);

        Ok(derive_device_status(
            operating_status,
            disc_present,
            position,
        ))
    }

    /// Enumerates the full disc contents into a structured [`Disc`]: title,
    /// capacity, and every track organized by group. Mirrors `listContent`
    /// (`netmd-commands.ts:178`).
    pub fn list_content(&self) -> Result<Disc> {
        debug!("list content");
        let flags = self.get_disc_flags()?;
        let title = self.get_disc_title(false)?;
        let full_width_title = self.get_disc_title(true)?;
        let capacity = self.get_disc_capacity()?;
        let track_count = self.get_track_count()?;

        let mut frames_used = time_to_frames(&capacity[0]);
        let mut frames_total = time_to_frames(&capacity[1]);
        let mut frames_left = time_to_frames(&capacity[2]);
        while frames_total > 512 * 60 * 82 {
            frames_used /= 2;
            frames_total /= 2;
            frames_left /= 2;
        }

        let mut disc = Disc {
            title,
            full_width_title,
            writable: flags.is_writable(),
            write_protected: flags.is_write_protected(),
            used: frames_used,
            left: frames_left,
            total: frames_total,
            track_count,
            groups: Vec::new(),
        };

        for (group_index, raw_group) in self.get_track_group_list()?.into_iter().enumerate() {
            let mut group = Group {
                index: group_index,
                title: raw_group.name,
                full_width_title: raw_group.full_width_name,
                tracks: Vec::new(),
            };
            for track_index in raw_group.tracks {
                let (encoding, channel) = self.get_track_encoding(track_index)?;
                let duration_frames = time_to_frames(&self.get_track_length(track_index)?);
                let protected = TrackFlag::from_byte(self.get_track_flags(track_index)?);
                group.tracks.push(Track {
                    index: track_index,
                    title: Some(self.get_track_title(track_index, false)?)
                        .filter(|s| !s.is_empty()),
                    full_width_title: Some(self.get_track_title(track_index, true)?)
                        .filter(|s| !s.is_empty()),
                    duration_frames,
                    channel,
                    encoding,
                    protected,
                });
            }
            disc.groups.push(group);
        }

        Ok(disc)
    }
}

fn playback_status2_to_operating_status(playback_status2: &[u8]) -> Option<u16> {
    match (playback_status2.get(4), playback_status2.get(5)) {
        (Some(&b1), Some(&b2)) => Some(((b1 as u16) << 8) | b2 as u16),
        _ => None,
    }
}

fn derive_device_status(
    operating_status: Option<u16>,
    disc_present: bool,
    position: Option<[u32; 5]>,
) -> DeviceStatus {
    let mut state = operating_status.map_or(PlaybackState::Unknown(0), PlaybackState::from_u16);

    if state == PlaybackState::Playing && !disc_present {
        state = PlaybackState::Ready;
    }

    let track = position.map(|p| p[0]);
    let time = position.map(|p| PlaybackTime {
        minute: p[2] + p[1] * 60,
        second: p[3],
        frame: p[4],
    });

    let disc_present_effective =
        disc_present && !matches!(state, PlaybackState::ReadingToc | PlaybackState::NoDisc);

    DeviceStatus {
        disc_present: disc_present_effective,
        state,
        track,
        time,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operating_status_word_from_block2() {
        let block = [0, 0, 0, 0, 0xc3, 0x75];
        assert_eq!(playback_status2_to_operating_status(&block), Some(0xc375));
        assert_eq!(playback_status2_to_operating_status(&[0, 1, 2]), None);
    }

    #[test]
    fn playback_state_mapping() {
        assert_eq!(PlaybackState::from_u16(50687), PlaybackState::Ready);
        assert_eq!(PlaybackState::from_u16(50037), PlaybackState::Playing);
        assert_eq!(PlaybackState::from_u16(50045), PlaybackState::Paused);
        assert_eq!(PlaybackState::from_u16(49983), PlaybackState::FastForward);
        assert_eq!(PlaybackState::from_u16(49999), PlaybackState::Rewind);
        assert_eq!(PlaybackState::from_u16(65315), PlaybackState::ReadingToc);
        assert_eq!(PlaybackState::from_u16(65296), PlaybackState::NoDisc);
        assert_eq!(PlaybackState::from_u16(65535), PlaybackState::DiscBlank);
        assert_eq!(
            PlaybackState::from_u16(65319),
            PlaybackState::ReadyForTransfer
        );
        assert_eq!(PlaybackState::from_u16(1234), PlaybackState::Unknown(1234));
    }

    #[test]
    fn playing_without_disc_becomes_ready() {
        let s = derive_device_status(Some(50037), false, None);
        assert_eq!(s.state, PlaybackState::Ready);
        assert!(!s.disc_present);
    }

    #[test]
    fn reading_toc_is_not_present() {
        let s = derive_device_status(Some(65315), true, None);
        assert_eq!(s.state, PlaybackState::ReadingToc);
        assert!(!s.disc_present);
    }

    #[test]
    fn position_maps_to_time_and_track() {
        let s = derive_device_status(Some(50037), true, Some([3, 1, 5, 30, 100]));
        assert_eq!(s.state, PlaybackState::Playing);
        assert!(s.disc_present);
        assert_eq!(s.track, Some(3));
        let t = s.time.unwrap();
        assert_eq!(t.minute, 5 + 60);
        assert_eq!(t.second, 30);
        assert_eq!(t.frame, 100);
    }

    #[test]
    fn no_position_yields_none_time() {
        let s = derive_device_status(Some(50687), true, None);
        assert_eq!(s.state, PlaybackState::Ready);
        assert_eq!(s.track, None);
        assert!(s.time.is_none());
    }
}
