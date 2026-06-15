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

/// Selects the ATRAC3 wire format from a parsed `fmt ` chunk, or `None` if the
/// chunk does not describe a 2-channel 44.1 kHz ATRAC3 stream.
///
/// Mirrors `getAtrac3Info` (`utils.ts:475`): mode is selected from `byteRate`.
fn atrac3_wireformat(fmt: &WavFmt) -> Option<Wireformat> {
    if fmt.format_tag != WAVE_FORMAT_SONY_ATRAC3 || fmt.channels != 2 {
        return None;
    }
    if fmt.sample_rate != 44100 {
        return None;
    }
    if fmt.byte_rate > 16000 {
        Some(Wireformat::Lp2)
    } else if fmt.byte_rate > 13000 {
        Some(Wireformat::L105kbps)
    } else if fmt.byte_rate > 8000 {
        Some(Wireformat::Lp4)
    } else {
        None
    }
}

/// Header-only ATRAC3 detection. Parses just the `fmt ` chunk — which appears
/// near the start of a RIFF/WAVE file — and returns the wire format if this is a
/// 2-channel 44.1 kHz ATRAC3 WAV.
///
/// Unlike [`atrac3_info`], this does **not** require the `data` chunk body to be
/// present, so it works on a truncated header prefix. Use it to cheaply probe a
/// large file before deciding whether to read it in full.
///
/// Returns a [`HeaderProbe`] distinguishing a definitive answer (the `fmt `
/// chunk was located within `header`) from an inconclusive one (the prefix is a
/// RIFF/WAVE file but `fmt ` lies beyond the supplied bytes), so the caller can
/// decide whether a full-file parse is warranted.
pub fn atrac3_format(header: &[u8]) -> HeaderProbe {
    match parse_wav_fmt(header) {
        FmtProbe::NotRiff => HeaderProbe::NotWav,
        FmtProbe::Found(fmt) => HeaderProbe::Found(atrac3_wireformat(&fmt)),
        FmtProbe::Truncated => HeaderProbe::Inconclusive,
    }
}

/// Outcome of a header-only ATRAC3 probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderProbe {
    /// The prefix is not a RIFF/WAVE file; it cannot be an ATRAC3 WAV.
    NotWav,
    /// The `fmt ` chunk was found within the prefix. The payload is
    /// `Some(format)` for ATRAC3 streams, `None` for any other WAV (e.g. PCM).
    Found(Option<Wireformat>),
    /// The prefix is RIFF/WAVE but `fmt ` lies beyond the supplied bytes, so a
    /// full-file parse is needed to decide.
    Inconclusive,
}

enum FmtProbe {
    NotRiff,
    Found(WavFmt),
    Truncated,
}

/// Parses only the `fmt ` chunk from a RIFF/WAVE prefix.
///
/// Walks the chunk list like [`parse_wav`] but stops as soon as `fmt ` is found
/// and does not require the `data` chunk (or any chunk body beyond `fmt `) to be
/// fully present. Distinguishes "not a WAV", "fmt found", and "fmt is beyond
/// the supplied prefix" so the caller can fall back to a full parse only when it
/// could actually change the answer.
fn parse_wav_fmt(data: &[u8]) -> FmtProbe {
    if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return FmtProbe::NotRiff;
    }
    let mut pos = 12usize;
    while pos + 8 <= data.len() {
        let id = &data[pos..pos + 4];
        let size = u32_le(data, pos + 4) as usize;
        let body_start = pos + 8;
        if id == b"fmt " {
            // The fmt chunk header is here but its body may be cut off.
            if size < 16 || body_start + 16 > data.len() {
                return FmtProbe::Truncated;
            }
            let b = &data[body_start..];
            return FmtProbe::Found(WavFmt {
                format_tag: u16_le(b, 0),
                channels: u16_le(b, 2),
                sample_rate: u32_le(b, 4),
                byte_rate: u32_le(b, 8),
                block_align: u16_le(b, 14),
            });
        }
        // Skip this chunk (word-aligned). Bail if it runs past the prefix.
        let advance = size + (size & 1);
        let next = match body_start.checked_add(advance) {
            Some(n) if n > pos => n,
            _ => return FmtProbe::Truncated,
        };
        pos = next;
    }
    // Ran out of prefix bytes before reaching `fmt `.
    FmtProbe::Truncated
}

/// If `data` is a 2-channel 44.1 kHz ATRAC3 WAV, returns its wire format and the
/// raw ATRAC3 payload (header stripped). Otherwise returns `None`.
pub fn atrac3_info(data: &[u8]) -> Option<(Wireformat, &[u8])> {
    let info = parse_wav(data).ok()?;
    let format = atrac3_wireformat(&info.fmt)?;
    let payload = &data[info.data_offset..info.data_offset + info.data_len];
    Some((format, payload))
}

/// Returns true if the WAV is plain PCM (used to decide whether to transcode for SP).
pub fn is_pcm(data: &[u8]) -> bool {
    parse_wav(data).is_ok_and(|i| i.fmt.format_tag == WAVE_FORMAT_PCM)
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
    fn header_only_probe_detects_truncated_atrac3() {
        // A full ATRAC3 WAV truncated to just past the fmt chunk (no data body)
        // must still be detected by the header-only probe.
        let wav = make_atrac3_wav(0x4099, &[0xAA; 1024]);
        // Keep RIFF(12) + fmt header(8) + fmt body(16) + a few bytes of the
        // data chunk header, but not the payload.
        let header = &wav[..12 + 8 + 16 + 4];
        assert_eq!(atrac3_format(header), HeaderProbe::Found(Some(Wireformat::Lp2)));
        // The full parser cannot succeed on this truncated prefix.
        assert!(atrac3_info(header).is_none());
    }

    #[test]
    fn header_only_probe_rejects_non_riff() {
        assert_eq!(atrac3_format(b"\x00\x00\x18ftypM4A "), HeaderProbe::NotWav);
        assert_eq!(atrac3_format(&[]), HeaderProbe::NotWav);
    }

    #[test]
    fn header_only_probe_found_non_atrac3_is_definitive() {
        // A PCM WAV's fmt is in the prefix: definitively not ATRAC3, no fallback.
        let mut wav = make_atrac3_wav(0x4099, &[0u8; 8]);
        wav[20] = 0x01; // format tag -> PCM
        wav[21] = 0x00;
        assert_eq!(atrac3_format(&wav), HeaderProbe::Found(None));
    }

    #[test]
    fn header_only_probe_inconclusive_when_fmt_past_prefix() {
        // A large JUNK chunk before fmt pushes fmt beyond an 8 KiB prefix.
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(b"WAVE");
        // JUNK chunk with a 9000-byte body.
        v.extend_from_slice(b"JUNK");
        v.extend_from_slice(&9000u32.to_le_bytes());
        v.extend_from_slice(&vec![0u8; 9000]);
        // fmt + ATRAC3 body + data, all past the 8 KiB mark.
        v.extend_from_slice(b"fmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&0x0270u16.to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        v.extend_from_slice(&44100u32.to_le_bytes());
        v.extend_from_slice(&0x4099u32.to_le_bytes());
        v.extend_from_slice(&0xC0u16.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&4u32.to_le_bytes());
        v.extend_from_slice(&[1, 2, 3, 4]);

        // 8 KiB prefix: cannot see fmt -> inconclusive.
        let prefix = &v[..8 * 1024];
        assert_eq!(atrac3_format(prefix), HeaderProbe::Inconclusive);
        // The full buffer parses successfully via the complete parser.
        let (fmt, _) = atrac3_info(&v).unwrap();
        assert_eq!(fmt, Wireformat::Lp2);
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
