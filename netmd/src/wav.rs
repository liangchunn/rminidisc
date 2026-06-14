//! WAV/RIFF parsing for upload data preparation.
//!
//! Detects ATRAC3 WAV files (format tag `0x0270`) and extracts the raw ATRAC3
//! `data` payload + wire format, mirroring `getAtrac3Info`
//! (`webminidisc/src/utils.ts:475`). Also provides PCM extraction for SP.
//!
//! Unlike the JS reference (which hardcodes chunk offsets), this walks the RIFF
//! chunk list generically so `fact`/`LIST` chunks are handled robustly.

use log::trace;

use crate::error::{NetMDError, Result};
use crate::types::Wireformat;

/// Sony ATRAC3 WAVE format tag.
const WAVE_FORMAT_SONY_ATRAC3: u16 = 0x0270;
/// Microsoft PCM WAVE format tag.
const WAVE_FORMAT_PCM: u16 = 0x0001;

/// A parsed WAVE `fmt ` chunk (only the fields we need).
#[derive(Debug, Clone, Copy)]
pub struct WavFmt {
    pub format_tag: u16,
    pub channels: u16,
    pub sample_rate: u32,
    pub byte_rate: u32,
    pub block_align: u16,
}

/// A parsed WAV file: its format chunk and the byte range of the `data` chunk.
#[derive(Debug, Clone)]
pub struct WavInfo {
    pub fmt: WavFmt,
    pub data_offset: usize,
    pub data_len: usize,
}

fn u16_le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32_le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// Parses a RIFF/WAVE file, locating the `fmt ` and `data` chunks.
pub fn parse_wav(data: &[u8]) -> Result<WavInfo> {
    trace!("parsing WAV: {} bytes", data.len());
    if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err(NetMDError::InvalidWav("not a RIFF/WAVE file".to_string()));
    }

    let mut fmt: Option<WavFmt> = None;
    let mut data_chunk: Option<(usize, usize)> = None;

    // Walk chunks starting after the 12-byte RIFF header.
    let mut pos = 12usize;
    while pos + 8 <= data.len() {
        let id = &data[pos..pos + 4];
        let size = u32_le(data, pos + 4) as usize;
        let body_start = pos + 8;
        if body_start + size > data.len() {
            // Truncated chunk; stop walking but keep what we have.
            break;
        }
        match id {
            b"fmt " => {
                if size >= 16 {
                    let b = &data[body_start..];
                    fmt = Some(WavFmt {
                        format_tag: u16_le(b, 0),
                        channels: u16_le(b, 2),
                        sample_rate: u32_le(b, 4),
                        byte_rate: u32_le(b, 8),
                        block_align: u16_le(b, 14),
                    });
                }
            }
            b"data" => {
                data_chunk = Some((body_start, size));
            }
            _ => {}
        }
        // Chunks are word-aligned (padded to even length).
        let advance = size + (size & 1);
        pos = body_start + advance;
        if data_chunk.is_some() && fmt.is_some() {
            break;
        }
    }

    let fmt = fmt.ok_or_else(|| NetMDError::InvalidWav("missing fmt chunk".to_string()))?;
    let (data_offset, data_len) =
        data_chunk.ok_or_else(|| NetMDError::InvalidWav("missing data chunk".to_string()))?;
    Ok(WavInfo {
        fmt,
        data_offset,
        data_len,
    })
}

/// If `data` is a 2-channel 44.1 kHz ATRAC3 WAV, returns its wire format and the
/// raw ATRAC3 payload (header stripped). Otherwise returns `None`.
///
/// Mirrors `getAtrac3Info` (`utils.ts:475`): mode is selected from `byteRate`.
pub fn atrac3_info(data: &[u8]) -> Option<(Wireformat, &[u8])> {
    let info = parse_wav(data).ok()?;
    if info.fmt.format_tag != WAVE_FORMAT_SONY_ATRAC3 || info.fmt.channels != 2 {
        return None;
    }
    if info.fmt.sample_rate != 44100 {
        return None;
    }
    let format = if info.fmt.byte_rate > 16000 {
        Wireformat::Lp2
    } else if info.fmt.byte_rate > 13000 {
        Wireformat::L105kbps
    } else if info.fmt.byte_rate > 8000 {
        Wireformat::Lp4
    } else {
        return None;
    };
    let payload = &data[info.data_offset..info.data_offset + info.data_len];
    Some((format, payload))
}

/// Returns true if the WAV is plain PCM (used to decide whether to transcode for SP).
pub fn is_pcm(data: &[u8]) -> bool {
    parse_wav(data)
        .map(|i| i.fmt.format_tag == WAVE_FORMAT_PCM)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Build a minimal ATRAC3 WAV header for a given byte_rate, with a data chunk.
    fn make_atrac3_wav(byte_rate: u32, data: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&(36u32 + data.len() as u32).to_le_bytes());
        v.extend_from_slice(b"WAVE");
        // fmt chunk (16 bytes minimal; real ATRAC3 is larger but 16 suffices here)
        v.extend_from_slice(b"fmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&0x0270u16.to_le_bytes()); // format tag
        v.extend_from_slice(&2u16.to_le_bytes()); // channels
        v.extend_from_slice(&44100u32.to_le_bytes()); // sample rate
        v.extend_from_slice(&byte_rate.to_le_bytes()); // byte rate
        v.extend_from_slice(&0xC0u16.to_le_bytes()); // block align
        v.extend_from_slice(&0u16.to_le_bytes()); // bits per sample
                                                  // data chunk
        v.extend_from_slice(b"data");
        v.extend_from_slice(&(data.len() as u32).to_le_bytes());
        v.extend_from_slice(data);
        v
    }

    #[test]
    fn detects_lp2() {
        let wav = make_atrac3_wav(0x4099, &[0xAA; 16]);
        let (fmt, payload) = atrac3_info(&wav).unwrap();
        assert_eq!(fmt, Wireformat::Lp2);
        assert_eq!(payload, &[0xAA; 16]);
    }

    #[test]
    fn detects_lp4() {
        let wav = make_atrac3_wav(0x204C, &[0xBB; 8]);
        let (fmt, payload) = atrac3_info(&wav).unwrap();
        assert_eq!(fmt, Wireformat::Lp4);
        assert_eq!(payload, &[0xBB; 8]);
    }

    #[test]
    fn rejects_non_atrac3() {
        // PCM file should not be detected as ATRAC3.
        let mut wav = make_atrac3_wav(0x4099, &[0; 8]);
        // Flip format tag to PCM.
        wav[20] = 0x01;
        wav[21] = 0x00;
        assert!(atrac3_info(&wav).is_none());
    }

    #[test]
    fn handles_fact_chunk() {
        // Insert a fact chunk before data.
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&100u32.to_le_bytes());
        v.extend_from_slice(b"WAVE");
        v.extend_from_slice(b"fmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&0x0270u16.to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        v.extend_from_slice(&44100u32.to_le_bytes());
        v.extend_from_slice(&0x4099u32.to_le_bytes());
        v.extend_from_slice(&0xC0u16.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(b"fact");
        v.extend_from_slice(&8u32.to_le_bytes());
        v.extend_from_slice(&[0u8; 8]);
        v.extend_from_slice(b"data");
        v.extend_from_slice(&4u32.to_le_bytes());
        v.extend_from_slice(&[1, 2, 3, 4]);
        let (fmt, payload) = atrac3_info(&v).unwrap();
        assert_eq!(fmt, Wireformat::Lp2);
        assert_eq!(payload, &[1, 2, 3, 4]);
    }
}
