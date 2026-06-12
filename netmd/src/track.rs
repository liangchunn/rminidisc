//! Track upload (download-to-device) orchestration.
//!
//! Ported from `MDTrack` / `MDSession` (`netmd-interface.ts:944-1153`) and
//! `download` (`netmd-commands.ts:465`). This is the track-WRITE direction
//! (host → device), which the MZ-N505 supports. Track-READ (device → host /
//! `saveTrackToArray`) is NOT ported (RH1/M200-only). See UNPORTED.md §3.

use log::debug;
use rusb::{DeviceHandle, UsbContext};

use crate::crypto::{encrypt_packets, retailmac};
use crate::ekb::get_ekb_for_device;
use crate::types::{DiscFormat, Wireformat, FRAME_SIZE};
use crate::{
    commit_track, enter_secure_session, get_leaf_id, leave_secure_session, prepare_download,
    release, send_key_data, send_track, session_key_exchange, session_key_forget, set_track_title,
    setup_download,
};

/// The hardcoded content ID used for uploads. Mirrors `MDTrack.getContentID`
/// (`netmd-interface.ts:992`).
const CONTENT_ID: [u8; 20] = [
    0x01, 0x0f, 0x50, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x48, 0xa2, 0x8d, 0x3e, 0x1a, 0x3b,
    0x0c, 0x44, 0xaf, 0x2f, 0xa0,
];

/// The hardcoded key-encryption-key. Mirrors `MDTrack.getKEK`
/// (`netmd-interface.ts:1000`).
const KEK: [u8; 8] = [0x14, 0xe3, 0x83, 0x4e, 0xe2, 0xd3, 0xcc, 0xa5];

/// Returns the on-device frame size in bytes for a wire format.
pub fn frame_size(format: Wireformat) -> usize {
    FRAME_SIZE
        .iter()
        .find(|(f, _)| *f == format)
        .map(|(_, s)| *s)
        .expect("unknown wire format")
}

/// Maps a wire format to its disc format. Mirrors `discforwire`
/// (`netmd-interface.ts:936`).
pub fn disc_for_wire(format: Wireformat) -> DiscFormat {
    match format {
        Wireformat::Pcm => DiscFormat::SpStereo,
        Wireformat::Lp2 => DiscFormat::Lp2,
        Wireformat::L105kbps => DiscFormat::Lp2,
        Wireformat::Lp4 => DiscFormat::Lp4,
    }
}

/// A track to upload to the device. Mirrors `MDTrack` (`netmd-interface.ts:944`).
pub struct MdTrack {
    /// Half-width title.
    pub title: String,
    /// Optional full-width title.
    pub full_width_title: Option<String>,
    /// The wire format of `data`.
    pub format: Wireformat,
    /// The raw (already encoded) audio payload. For SP this is big-endian PCM;
    /// for LP2/LP4 this is raw ATRAC3 (WAV header already stripped).
    pub data: Vec<u8>,
}

impl MdTrack {
    /// Total payload size, padded up to a whole number of frames.
    pub fn total_size(&self) -> usize {
        let fs = frame_size(self.format);
        let len = self.data.len();
        if len % fs != 0 {
            len + (fs - (len % fs))
        } else {
            len
        }
    }

    /// Number of frames in the (padded) payload.
    pub fn frame_count(&self) -> u32 {
        (self.total_size() / frame_size(self.format)) as u32
    }
}

/// Uploads a track to the device.
///
/// Mirrors `download` (`netmd-commands.ts:465`) + `MDSession.downloadTrack`
/// (`netmd-interface.ts:1118`). Returns `(track_number, uuid_hex, content_id_hex)`.
pub fn download_track<T: UsbContext>(
    handle: &DeviceHandle<T>,
    track: &MdTrack,
    vendor: u16,
    product: u16,
    progress: Option<&mut dyn FnMut(u64, u64)>,
) -> anyhow::Result<(u16, String, String)> {
    prepare_download(handle)?;

    // --- MDSession.init ---
    enter_secure_session(handle)?;
    let leaf_id = get_leaf_id(handle)?;
    let ekb = get_ekb_for_device(&leaf_id, vendor, product);
    send_key_data(handle, ekb.ekb_id, &ekb.key_chain, ekb.depth, &ekb.signature)?;

    let mut host_nonce = [0u8; 8];
    {
        use rand::RngCore;
        rand::thread_rng().fill_bytes(&mut host_nonce);
    }
    let dev_nonce = session_key_exchange(handle, &host_nonce)?;
    let mut nonce = [0u8; 16];
    nonce[..8].copy_from_slice(&host_nonce);
    nonce[8..].copy_from_slice(&dev_nonce);
    let session_key = retailmac(&ekb.root_key, &nonce, &[0u8; 8]);
    debug!("session established");

    // --- MDSession.downloadTrack ---
    let result = download_track_inner(handle, track, &session_key, progress);

    // --- MDSession.close (always run, even on error) ---
    let _ = session_key_forget(handle);
    let _ = leave_secure_session(handle);
    let _ = release(handle);

    result
}

fn download_track_inner<T: UsbContext>(
    handle: &DeviceHandle<T>,
    track: &MdTrack,
    session_key: &[u8; 8],
    progress: Option<&mut dyn FnMut(u64, u64)>,
) -> anyhow::Result<(u16, String, String)> {
    setup_download(handle, &CONTENT_ID, &KEK, session_key)?;

    let packets = encrypt_packets(&KEK, frame_size(track.format), &track.data);
    let (track_num, uuid, ccid) = send_track(
        handle,
        track.format as u8,
        disc_for_wire(track.format) as u8,
        track.frame_count(),
        track.total_size() as u32,
        &packets,
        session_key,
        progress,
    )?;

    set_track_title(handle, track_num, &track.title, false)?;
    if let Some(fw) = &track.full_width_title {
        set_track_title(handle, track_num, fw, true)?;
    }
    commit_track(handle, track_num, session_key)?;
    Ok((track_num, uuid, ccid))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn total_size_pads_to_frame() {
        let t = MdTrack {
            title: "t".into(),
            full_width_title: None,
            format: Wireformat::Lp4, // frame size 96
            data: vec![0u8; 100],
        };
        assert_eq!(t.total_size(), 192);
        assert_eq!(t.frame_count(), 2);
    }

    #[test]
    fn total_size_exact_multiple() {
        let t = MdTrack {
            title: "t".into(),
            full_width_title: None,
            format: Wireformat::Pcm, // frame size 2048
            data: vec![0u8; 4096],
        };
        assert_eq!(t.total_size(), 4096);
        assert_eq!(t.frame_count(), 2);
    }

    #[test]
    fn disc_for_wire_mapping() {
        assert_eq!(disc_for_wire(Wireformat::Pcm), DiscFormat::SpStereo);
        assert_eq!(disc_for_wire(Wireformat::Lp2), DiscFormat::Lp2);
        assert_eq!(disc_for_wire(Wireformat::Lp4), DiscFormat::Lp4);
    }
}
