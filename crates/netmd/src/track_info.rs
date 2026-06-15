use log::{debug, info, trace};

use crate::{
    descriptor::{Descriptor, DescriptorAction},
    error::{NetMDError, Result},
    scan::scan,
    title::{sanitize_full_width_title, sanitize_half_width_title},
    types::{ChannelCount, Encoding},
    util::{encode_to_sjis, get_length_after_sjis_encode, parse_bcd_u8, parse_string, parse_u8},
};

use super::NetMD;

impl NetMD {
    /// Reads a track's title. Mirrors `NetMDInterface.getTrackTitle`.
    ///
    /// `track` is 0-based. `w_char` selects the full-width title (UTOC4) vs the
    /// half-width title (UTOC1).
    pub fn get_track_title(&self, track: u16, w_char: bool) -> Result<String> {
        trace!("get track title #{track} (wchar={w_char})");
        let wchar_value: u8 = if w_char { 3 } else { 2 };
        let descriptor = if w_char {
            Descriptor::AudioUtoc4Td
        } else {
            Descriptor::AudioUtoc1Td
        };
        self.change_descriptor_state(descriptor, DescriptorAction::OpenRead)?;
        let query = format!(
            "00 1806 022018{:02x} {:04x} 3000 0a00 ff00 00000000",
            wchar_value, track
        );
        let reply = self.send_query(query)?;
        let data = reply.scan("%? 1806 022018%? %?%? %?%? %?%? 1000 00%?0000 00%?000a %x")?;
        let title = parse_string(data[0])?;
        self.change_descriptor_state(descriptor, DescriptorAction::Close)?;
        Ok(title)
    }

    pub fn get_track_count(&self) -> Result<u8> {
        debug!("get track count");
        self.change_descriptor_state(Descriptor::AudioContentsTd, DescriptorAction::OpenRead)?;
        let reply = self.send_query("00 1806 02101001 3000 1000 ff00 00000000")?;

        let data = reply.scan("%? 1806 02101001 %?%? %?%? 1000 00%?0000 0006 0010000200%b")?;

        let [tc] = &data[..] else {
            return Err(NetMDError::UnexpectedResponse(
                "unexpected scan result count".to_string(),
            ));
        };
        let track_count = parse_u8(tc)?;
        self.change_descriptor_state(Descriptor::AudioContentsTd, DescriptorAction::Close)?;
        Ok(track_count)
    }

    /// Sets a track's title. Mirrors `NetMDInterface.setTrackTitle`. `track` is 0-based.
    pub fn set_track_title(&self, track: u16, title: &str, w_char: bool) -> Result<()> {
        debug!("set track title #{track}: {title:?} (wchar={w_char})");
        let title = sanitize_title(title, w_char);
        let wchar_value: u8 = if w_char { 3 } else { 2 };
        let descriptor = if w_char {
            Descriptor::AudioUtoc4Td
        } else {
            Descriptor::AudioUtoc1Td
        };

        let new_len = get_length_after_sjis_encode(&title)?;
        let old_len = match self.get_track_title(track, w_char) {
            Ok(current) => {
                if current == title.as_str() {
                    return Ok(());
                }
                get_length_after_sjis_encode(&current)?
            }
            Err(_) => 0,
        };

        let sjis = encode_to_sjis(&title)?;
        let sjis_hex = sjis.iter().map(|b| format!("{b:02x}")).collect::<String>();

        self.change_descriptor_state(descriptor, DescriptorAction::OpenWrite)?;
        let query = format!(
            "00 1807 022018{:02x} {:04x} 3000 0a00 5000 {:04x} 0000 {:04x} {}",
            wchar_value, track, new_len, old_len, sjis_hex
        );
        let reply = self.send_query(query)?;
        reply.scan("%? 1807 022018%? %?%? 3000 0a00 5000 %?%? 0000 %?%?")?;
        self.change_descriptor_state(descriptor, DescriptorAction::Close)?;
        Ok(())
    }

    /// Reads raw per-track info bytes. Mirrors `NetMDInterface._getTrackInfo`.
    /// `track` is 0-based.
    pub fn get_track_info(&self, track: u16, p1: u16, p2: u16) -> Result<Vec<u8>> {
        trace!("get track info #{track} (p1=0x{p1:04x} p2=0x{p2:04x})");
        self.change_descriptor_state(Descriptor::AudioContentsTd, DescriptorAction::OpenRead)?;
        let query = format!(
            "00 1806 02201001 {:04x} {:04x} {:04x} ff00 00000000",
            track, p1, p2
        );
        let reply = self.send_query(query)?;
        let data = reply.scan("%? 1806 02201001 %?%? %?%? %?%? 1000 00%?0000 %x")?;
        let raw = data[0].to_vec();
        self.change_descriptor_state(Descriptor::AudioContentsTd, DescriptorAction::Close)?;
        Ok(raw)
    }

    /// Reads a track's length as `[h, m, s, f]`. Mirrors `getTrackLength`.
    pub fn get_track_length(&self, track: u16) -> Result<[u32; 4]> {
        trace!("get track length #{track}");
        let raw = self.get_track_info(track, 0x3000, 0x0100)?;
        let data = scan("0001 0006 0000 %B %B %B %B", &raw)?;
        Ok([
            parse_bcd_u8(data[0])? as u32,
            parse_bcd_u8(data[1])? as u32,
            parse_bcd_u8(data[2])? as u32,
            parse_bcd_u8(data[3])? as u32,
        ])
    }

    /// Reads a track's `(encoding, channel_count)`. Mirrors `getTrackEncoding`.
    pub fn get_track_encoding(&self, track: u16) -> Result<(Encoding, ChannelCount)> {
        trace!("get track encoding #{track}");
        let raw = self.get_track_info(track, 0x3080, 0x0700)?;
        let data = scan("8007 0004 0110 %b %b", &raw)?;
        Ok((
            track_encoding_from_byte(parse_u8(data[0])?)?,
            channel_count_from_byte(parse_u8(data[1])?)?,
        ))
    }

    /// Reads a track's flags. Mirrors `NetMDInterface.getTrackFlags`. `track` is 0-based.
    pub fn get_track_flags(&self, track: u16) -> Result<u8> {
        trace!("get track flags #{track}");
        self.change_descriptor_state(Descriptor::AudioContentsTd, DescriptorAction::OpenRead)?;
        let query = format!("00 1806 01201001 {:04x} ff00 00010008", track);
        let reply = self.send_query(query)?;
        let data = reply.scan("%? 1806 01201001 %?%? 10 00 00010008 %b")?;
        let flags = parse_u8(data[0])?;
        self.change_descriptor_state(Descriptor::AudioContentsTd, DescriptorAction::Close)?;
        Ok(flags)
    }

    /// Erases a single track. Mirrors `NetMDInterface.eraseTrack`. `track` is 0-based.
    /// DESTRUCTIVE.
    pub fn erase_track(&self, track: u16) -> Result<()> {
        info!("erase track #{track}");
        let query = format!("00 1840 ff01 00 201001 {:04x}", track);
        self.send_query(query)?;
        Ok(())
    }

    /// Moves a track from `source` to `dest` (0-based). Mirrors `NetMDInterface.moveTrack`.
    /// DESTRUCTIVE (reorders disc).
    pub fn move_track(&self, source: u16, dest: u16) -> Result<()> {
        info!("move track {source} -> {dest}");
        let query = format!("00 1843 ff00 00 201001 {:04x} 201001 {:04x}", source, dest);
        self.send_query(query)?;
        Ok(())
    }

    /// Erases the entire disc. Mirrors `NetMDInterface.eraseDisc`. DESTRUCTIVE.
    pub fn erase_disc(&self) -> Result<()> {
        info!("erase disc");
        let reply = self.send_query("00 1840 ff 0000")?;
        reply.scan("%? 1840 00 0000")?;
        Ok(())
    }
}

fn sanitize_title(title: &str, w_char: bool) -> String {
    if w_char {
        sanitize_full_width_title(title)
    } else {
        sanitize_half_width_title(title)
    }
}

fn track_encoding_from_byte(value: u8) -> Result<Encoding> {
    match value {
        value if value == Encoding::Sp as u8 => Ok(Encoding::Sp),
        value if value == Encoding::Lp2 as u8 => Ok(Encoding::Lp2),
        value if value == Encoding::Lp4 as u8 => Ok(Encoding::Lp4),
        _ => Err(NetMDError::UnexpectedResponse(format!(
            "unknown track encoding: 0x{value:02x}"
        ))),
    }
}

fn channel_count_from_byte(value: u8) -> Result<ChannelCount> {
    match value {
        0x00 => Ok(ChannelCount::Stereo),
        0x01 => Ok(ChannelCount::Mono),
        _ => Err(NetMDError::UnexpectedResponse(format!(
            "unknown channel count: 0x{value:02x}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_title_selects_width_specific_sanitizer() {
        assert_eq!(sanitize_title("ソニー", false), "ｿﾆ-");
        assert_eq!(sanitize_title("Sony 1", true), "Ｓｏｎｙ　１");
    }

    #[test]
    fn track_encoding_decodes_wire_values() {
        assert_eq!(track_encoding_from_byte(0x90).unwrap(), Encoding::Sp);
        assert_eq!(track_encoding_from_byte(0x92).unwrap(), Encoding::Lp2);
        assert_eq!(track_encoding_from_byte(0x93).unwrap(), Encoding::Lp4);
        assert!(track_encoding_from_byte(0xff).is_err());
    }

    #[test]
    fn channel_count_decodes_wire_values() {
        assert_eq!(channel_count_from_byte(0x00).unwrap(), ChannelCount::Stereo);
        assert_eq!(channel_count_from_byte(0x01).unwrap(), ChannelCount::Mono);
        assert!(channel_count_from_byte(0xff).is_err());
    }
}
