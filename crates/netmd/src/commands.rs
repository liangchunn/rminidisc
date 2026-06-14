//! High-level convenience commands composed from the lower-level protocol
//! functions. Ports selected helpers from `netmd-commands.ts`.

use log::debug;
use rusb::{DeviceHandle, UsbContext};

use crate::{
    error::Result,
    playback::{get_playback_status2, get_position},
    status::get_status,
    types::{DeviceStatus, PlaybackState, PlaybackTime},
};

/// Returns a comprehensive playback status snapshot. Mirrors
/// `getDeviceStatus` (`netmd-commands.ts:131`).
pub fn get_device_status<T: UsbContext>(handle: &DeviceHandle<T>) -> Result<DeviceStatus> {
    debug!("get device status");
    let status = get_status(handle)?;
    let playback_status2 = get_playback_status2(handle)?;
    let position = get_position(handle)?;

    let operating_status = playback_status2_to_operating_status(&playback_status2);
    let disc_present = status.get(4) != Some(&0x80);

    Ok(derive_device_status(
        operating_status,
        disc_present,
        position,
    ))
}

/// Extracts the 16-bit operating status from playback-status block 2.
/// Mirrors `(playbackStatus2[4] << 8) | playbackStatus2[5]`
/// (`netmd-commands.ts:134`).
fn playback_status2_to_operating_status(playback_status2: &[u8]) -> Option<u16> {
    match (playback_status2.get(4), playback_status2.get(5)) {
        (Some(&b1), Some(&b2)) => Some(((b1 as u16) << 8) | b2 as u16),
        _ => None,
    }
}

/// Pure derivation of [`DeviceStatus`] from the raw inputs, separated for
/// testing. Mirrors the corrections in `netmd-commands.ts:140-159`.
fn derive_device_status(
    operating_status: Option<u16>,
    disc_present: bool,
    position: Option<[u32; 5]>,
) -> DeviceStatus {
    let mut state = operating_status
        .map(PlaybackState::from_u16)
        .unwrap_or(PlaybackState::Unknown(0));

    // "playing" without a disc actually means "ready".
    if state == PlaybackState::Playing && !disc_present {
        state = PlaybackState::Ready;
    }

    let track = position.map(|p| p[0]);
    let time = position.map(|p| PlaybackTime {
        // position = [track, hour, minute, second, frame]
        minute: p[2] + p[1] * 60,
        second: p[3],
        frame: p[4],
    });

    // While reading the TOC or with no disc, treat as "not present".
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
        // position = [track, hour, minute, second, frame]
        let s = derive_device_status(Some(50037), true, Some([3, 1, 5, 30, 100]));
        assert_eq!(s.state, PlaybackState::Playing);
        assert!(s.disc_present);
        assert_eq!(s.track, Some(3));
        let t = s.time.unwrap();
        assert_eq!(t.minute, 5 + 60); // hour*60 + minute
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
