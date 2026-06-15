//! Parsing and formatting helpers shared across protocol modules.
//!
//! Byte/BCD parsing ([`parse_u8`], [`parse_u16`], [`parse_bcd_u8`], ...),
//! Shift-JIS string encode/decode for titles, and NetMD frame/time conversion
//! ([`time_to_frames`], [`format_time_from_frames`]).

use std::array::TryFromSliceError;

use crate::error::NetMDError;

pub(crate) fn parse_u16(buf: &[u8]) -> Result<u16, TryFromSliceError> {
    Ok(u16::from_be_bytes(<[u8; 2]>::try_from(buf)?))
}

pub(crate) fn parse_u8(buf: &[u8]) -> Result<u8, TryFromSliceError> {
    <[u8; 1]>::try_from(buf).map(|b| b[0])
}

#[allow(dead_code)] // ported helper, kept for parity with netmd-js
pub(crate) fn parse_u32(buf: &[u8]) -> Result<u32, TryFromSliceError> {
    Ok(u32::from_be_bytes(<[u8; 4]>::try_from(buf)?))
}

pub(crate) fn parse_string(buf: &[u8]) -> crate::error::Result<String> {
    let (title, _encoding, errors) = encoding_rs::SHIFT_JIS.decode(buf);

    if errors {
        return Err(NetMDError::TextEncoding(
            "invalid SHIFT_JIS string".to_string(),
        ));
    }

    Ok(title.to_string())
}

pub(crate) fn encode_to_sjis(utf8: &str) -> crate::error::Result<Vec<u8>> {
    let (encoded, _encoding, errors) = encoding_rs::SHIFT_JIS.encode(utf8);
    if errors {
        return Err(NetMDError::TextEncoding(
            "invalid UTF-8 for SHIFT_JIS encoding".to_string(),
        ));
    }
    Ok(encoded.into_owned())
}

pub(crate) fn get_length_after_sjis_encode(utf8: &str) -> crate::error::Result<usize> {
    Ok(encode_to_sjis(utf8)?.len())
}

pub(crate) fn bcd_to_int(mut bcd: u16) -> u16 {
    let mut value: u16 = 0;
    let mut nibble: u32 = 0;
    while bcd != 0 {
        let nibble_value = bcd & 0x0f;
        bcd >>= 4;
        value += nibble_value * 10u16.pow(nibble);
        nibble += 1;
    }
    value
}

pub(crate) fn parse_bcd_u8(buf: &[u8]) -> Result<u8, TryFromSliceError> {
    let val = parse_u8(buf)?;
    Ok(bcd_to_int(val as u16) as u8)
}

pub(crate) fn parse_bcd_u16(buf: &[u8]) -> Result<u16, TryFromSliceError> {
    let val = parse_u16(buf)?;
    Ok(bcd_to_int(val))
}

pub fn format_time_from_frames(value: u32) -> String {
    let f = value % 512;
    let value = (value - f) / 512;
    let s = value % 60;
    let value = (value - s) / 60;
    let m = value % 60;
    let h = (value - m) / 60;
    format!("{:02}:{:02}:{:02}+{:03}", h, m, s, f)
}

pub fn time_to_frames(time: &[u32; 4]) -> u32 {
    ((time[0] * 60 + time[1]) * 60 + time[2]) * 512 + time[3]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_big_endian_integers() {
        assert_eq!(parse_u8(&[0x12]).unwrap(), 0x12);
        assert_eq!(parse_u16(&[0x12, 0x34]).unwrap(), 0x1234);
        assert_eq!(parse_u32(&[0x12, 0x34, 0x56, 0x78]).unwrap(), 0x12345678);
    }

    #[test]
    fn parse_integers_reject_wrong_length() {
        assert!(parse_u8(&[]).is_err());
        assert!(parse_u16(&[0x12]).is_err());
        assert!(parse_u16(&[0x12, 0x34, 0x56]).is_err());
        assert!(parse_u32(&[0x12, 0x34, 0x56]).is_err());
    }

    #[test]
    fn bcd_decoding() {
        // BCD byte: each nibble is a decimal digit.
        assert_eq!(bcd_to_int(0x00), 0);
        assert_eq!(bcd_to_int(0x10), 10);
        assert_eq!(bcd_to_int(0x42), 42);
        assert_eq!(bcd_to_int(0x59), 59);
        // BCD word.
        assert_eq!(bcd_to_int(0x0608), 608);
        assert_eq!(bcd_to_int(0x1234), 1234);
    }

    #[test]
    fn parse_bcd_slices() {
        // Mirrors disc-capacity decoding: %W (word) + %B (byte) values.
        assert_eq!(parse_bcd_u8(&[0x35]).unwrap(), 35);
        assert_eq!(parse_bcd_u8(&[0x34]).unwrap(), 34);
        assert_eq!(parse_bcd_u16(&[0x06, 0x08]).unwrap(), 608);
    }

    #[test]
    fn format_time_known_value() {
        // 06:52+042 of track length (h=0): seconds=6*60+52=412, frames=42.
        let frames = (6 * 60 + 52) * 512 + 42;
        assert_eq!(format_time_from_frames(frames), "00:06:52+042");
    }

    #[test]
    fn format_time_zero() {
        assert_eq!(format_time_from_frames(0), "00:00:00+000");
    }

    #[test]
    fn time_frames_round_trip() {
        for t in [
            [0, 6, 52, 42],
            [1, 17, 35, 34],
            [0, 0, 0, 0],
            [2, 59, 59, 511],
        ] {
            let frames = time_to_frames(&t);
            let formatted = format_time_from_frames(frames);
            let expected = format!("{:02}:{:02}:{:02}+{:03}", t[0], t[1], t[2], t[3]);
            assert_eq!(formatted, expected, "round-trip failed for {t:?}");
        }
    }

    #[test]
    fn sjis_round_trip_ascii() {
        let s = "Paradis - Toi Et Moi";
        let encoded = encode_to_sjis(s).unwrap();
        assert_eq!(encoded.len(), s.len()); // ASCII is 1 byte each in SJIS
        assert_eq!(get_length_after_sjis_encode(s).unwrap(), s.len());
        assert_eq!(parse_string(&encoded).unwrap(), s);
    }

    #[test]
    fn sjis_round_trip_japanese() {
        // Half-width katakana / kanji encode to multiple bytes in SJIS.
        let s = "ソニー";
        let encoded = encode_to_sjis(s).unwrap();
        assert_eq!(encoded.len(), 6); // 3 double-byte chars
        assert_eq!(parse_string(&encoded).unwrap(), s);
    }

    #[test]
    fn sjis_rejects_unrepresentable() {
        // Characters with no SJIS mapping must error rather than silently drop.
        assert!(encode_to_sjis("😀").is_err());
    }

    #[test]
    fn parse_string_decodes_empty() {
        assert_eq!(parse_string(&[]).unwrap(), "");
    }
}
