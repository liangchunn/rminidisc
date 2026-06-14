use log::{debug, info, trace};
use rusb::UsbContext;

use crate::crypto::{encrypt_packets, retailmac};
use crate::ekb::get_ekb_for_device;
use crate::error::Result;
use crate::types::{DiscFormat, Wireformat, FRAME_SIZE};

use super::NetMD;

const CONTENT_ID: [u8; 20] = [
    0x01, 0x0f, 0x50, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x48, 0xa2, 0x8d, 0x3e, 0x1a, 0x3b, 0x0c,
    0x44, 0xaf, 0x2f, 0xa0,
];

const KEK: [u8; 8] = [0x14, 0xe3, 0x83, 0x4e, 0xe2, 0xd3, 0xcc, 0xa5];

pub fn frame_size(format: Wireformat) -> usize {
    FRAME_SIZE
        .iter()
        .find(|(f, _)| *f == format)
        .map(|(_, s)| *s)
        .expect("unknown wire format")
}

pub fn disc_for_wire(format: Wireformat) -> DiscFormat {
    match format {
        Wireformat::Pcm => DiscFormat::SpStereo,
        Wireformat::Lp2 => DiscFormat::Lp2,
        Wireformat::L105kbps => DiscFormat::Lp2,
        Wireformat::Lp4 => DiscFormat::Lp4,
    }
}

pub struct MdTrack {
    pub title: String,
    pub full_width_title: Option<String>,
    pub format: Wireformat,
    pub data: Vec<u8>,
}

impl MdTrack {
    pub fn total_size(&self) -> usize {
        let fs = frame_size(self.format);
        let len = self.data.len();
        if !len.is_multiple_of(fs) {
            len + (fs - (len % fs))
        } else {
            len
        }
    }

    pub fn frame_count(&self) -> u32 {
        (self.total_size() / frame_size(self.format)) as u32
    }
}

impl<T: UsbContext> NetMD<T> {
    /// Uploads a track to the device.
    ///
    /// Mirrors `download` (`netmd-commands.ts:465`) + `MDSession.downloadTrack`
    /// (`netmd-interface.ts:1118`). Returns `(track_number, uuid_hex, content_id_hex)`.
    pub fn download_track(
        &self,
        track: &MdTrack,
        progress: Option<&mut dyn FnMut(u64, u64)>,
    ) -> Result<(u16, String, String)> {
        info!(
            "downloading track '{}' (format={:?}, {} frames, {} bytes)",
            track.title,
            track.format,
            track.frame_count(),
            track.total_size(),
        );
        self.prepare_download()?;

        self.enter_secure_session()?;
        let leaf_id = self.get_leaf_id()?;
        let ekb = get_ekb_for_device(&leaf_id, self.vendor_id, self.product_id);
        self.send_key_data(ekb.ekb_id, &ekb.key_chain, ekb.depth, &ekb.signature)?;

        let mut host_nonce = [0u8; 8];
        {
            use rand::Rng;
            rand::rng().fill_bytes(&mut host_nonce);
        }
        let dev_nonce = self.session_key_exchange(&host_nonce)?;
        let mut nonce = [0u8; 16];
        nonce[..8].copy_from_slice(&host_nonce);
        nonce[8..].copy_from_slice(&dev_nonce);
        let session_key = retailmac(&ekb.root_key, &nonce, &[0u8; 8]);
        debug!("session established");

        let result = self.download_track_inner(track, &session_key, progress);

        let _ = self.session_key_forget();
        let _ = self.leave_secure_session();
        let _ = self.release();

        match &result {
            Ok((track_num, uuid, _)) => {
                info!("track #{track_num} uploaded successfully (uuid={uuid})");
            }
            Err(e) => {
                info!("track upload failed: {e}");
            }
        }
        result
    }

    fn download_track_inner(
        &self,
        track: &MdTrack,
        session_key: &[u8; 8],
        progress: Option<&mut dyn FnMut(u64, u64)>,
    ) -> Result<(u16, String, String)> {
        trace!("setting up download");
        self.setup_download(&CONTENT_ID, &KEK, session_key)?;

        trace!(
            "encrypting packets (frame_size={})",
            frame_size(track.format)
        );
        let packets = encrypt_packets(&KEK, frame_size(track.format), &track.data);
        trace!("sending encrypted track");
        let (track_num, uuid, ccid) = self.send_track(
            track.format as u8,
            disc_for_wire(track.format) as u8,
            track.frame_count(),
            track.total_size() as u32,
            &packets,
            session_key,
            progress,
        )?;

        trace!("setting track title");
        self.set_track_title(track_num, &track.title, false)?;
        if let Some(fw) = &track.full_width_title {
            self.set_track_title(track_num, fw, true)?;
        }
        trace!("committing track #{track_num}");
        self.commit_track(track_num, session_key)?;
        Ok((track_num, uuid, ccid))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn total_size_pads_to_frame() {
        let t = MdTrack {
            title: "t".into(),
            full_width_title: None,
            format: Wireformat::Lp4,
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
            format: Wireformat::Pcm,
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
