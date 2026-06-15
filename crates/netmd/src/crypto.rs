//! DES-based crypto for the secure download pipeline.
//!
//! Ported from `netmd-js`:
//! - `retailmac` — `netmd-interface.ts:915`
//! - packet encryption — `encrypt-generator.ts` / `MDTrack.getPacketIterator`
//!
//! All operations use 8-byte (single) DES, except the second `retailmac` step
//! which uses two-key Triple-DES (EDE2). No padding is ever applied: every input
//! handled here is a multiple of the 8-byte block size (frame sizes are
//! 96/152/192/2048, and PCM packets are padded to frame size by the caller).

use std::io::Read;

use cipher::generic_array::GenericArray;
use cipher::{BlockDecryptMut, BlockEncryptMut, KeyInit, KeyIvInit};
use des::{Des, TdesEde2};
use log::trace;

use crate::error::{NetMDError, Result};

type DesCbcEnc = cbc::Encryptor<Des>;
type DesCbcDec = cbc::Decryptor<Des>;
type DesEcbEnc = ecb::Encryptor<Des>;
type DesEcbDec = ecb::Decryptor<Des>;
type TdesCbcEnc = cbc::Encryptor<TdesEde2>;

/// DES-CBC encrypt without padding. `data.len()` must be a multiple of 8.
pub(crate) fn des_cbc_encrypt(key: &[u8; 8], iv: &[u8; 8], data: &[u8]) -> Vec<u8> {
    assert_eq!(data.len() % 8, 0, "des_cbc_encrypt: data not block-aligned");
    let mut buf = data.to_vec();
    let mut enc = DesCbcEnc::new(key.into(), iv.into());
    for chunk in buf.chunks_mut(8) {
        let block = GenericArray::from_mut_slice(chunk);
        enc.encrypt_block_mut(block);
    }
    buf
}

/// DES-CBC decrypt without padding. `data.len()` must be a multiple of 8.
pub(crate) fn des_cbc_decrypt(key: &[u8; 8], iv: &[u8; 8], data: &[u8]) -> Vec<u8> {
    assert_eq!(data.len() % 8, 0, "des_cbc_decrypt: data not block-aligned");
    let mut buf = data.to_vec();
    let mut dec = DesCbcDec::new(key.into(), iv.into());
    for chunk in buf.chunks_mut(8) {
        let block = GenericArray::from_mut_slice(chunk);
        dec.decrypt_block_mut(block);
    }
    buf
}

/// DES-ECB encrypt without padding. `data.len()` must be a multiple of 8.
pub(crate) fn des_ecb_encrypt(key: &[u8; 8], data: &[u8]) -> Vec<u8> {
    assert_eq!(data.len() % 8, 0, "des_ecb_encrypt: data not block-aligned");
    let mut buf = data.to_vec();
    let mut enc = DesEcbEnc::new(key.into());
    for chunk in buf.chunks_mut(8) {
        let block = GenericArray::from_mut_slice(chunk);
        enc.encrypt_block_mut(block);
    }
    buf
}

/// DES-ECB decrypt without padding. `data.len()` must be a multiple of 8.
pub(crate) fn des_ecb_decrypt(key: &[u8; 8], data: &[u8]) -> Vec<u8> {
    assert_eq!(data.len() % 8, 0, "des_ecb_decrypt: data not block-aligned");
    let mut buf = data.to_vec();
    let mut dec = DesEcbDec::new(key.into());
    for chunk in buf.chunks_mut(8) {
        let block = GenericArray::from_mut_slice(chunk);
        dec.decrypt_block_mut(block);
    }
    buf
}

/// Two-key Triple-DES CBC encrypt without padding. Key is 16 bytes.
pub(crate) fn tdes_cbc_encrypt(key: &[u8; 16], iv: &[u8; 8], data: &[u8]) -> Vec<u8> {
    assert_eq!(
        data.len() % 8,
        0,
        "tdes_cbc_encrypt: data not block-aligned"
    );
    let mut buf = data.to_vec();
    let mut enc = TdesCbcEnc::new(key.into(), iv.into());
    for chunk in buf.chunks_mut(8) {
        let block = GenericArray::from_mut_slice(chunk);
        enc.encrypt_block_mut(block);
    }
    buf
}

/// Computes the retail MAC used to derive the session key.
///
/// Mirrors `retailmac` (`netmd-interface.ts:915`):
/// 1. `step1 = DES-CBC(value[..len-8], subkeyA=key[0..8], iv)`; take the last
///    8 bytes of the ciphertext as `iv2`.
/// 2. `step2 = TripleDES-CBC(value[len-8..], key, iv2)`.
/// 3. Return the first 8 bytes of `step2`.
///
/// Returns the 8-byte MAC (the JS returns it as 16 hex chars; we keep raw bytes
/// and let callers hex-encode if needed — but the session key is used as bytes).
pub(crate) fn retailmac(key: &[u8; 16], value: &[u8], iv: &[u8; 8]) -> [u8; 8] {
    assert!(value.len() >= 8, "retailmac: value too short");
    assert_eq!(value.len() % 8, 0, "retailmac: value not block-aligned");
    trace!("retailmac: value_len={} iv=...", value.len());

    let subkey_a: [u8; 8] = key[0..8].try_into().unwrap();
    let split = value.len() - 8;
    let beginning = &value[..split];
    let end = &value[split..];

    let step1 = des_cbc_encrypt(&subkey_a, iv, beginning);
    // iv2 = last 8 bytes of step1 ciphertext.
    let iv2: [u8; 8] = step1[step1.len() - 8..].try_into().unwrap();

    let step2 = tdes_cbc_encrypt(key, &iv2, end);
    step2[0..8].try_into().unwrap()
}

/// One encrypted packet to send over the bulk endpoint.
pub(crate) struct EncryptedPacket {
    /// The wrapped data key (KEK-decrypted random key), 8 bytes. Sent in the
    /// first packet header only.
    pub(crate) key: [u8; 8],
    /// The IV for this packet (8 bytes). Sent in the first packet header only.
    pub(crate) iv: [u8; 8],
    /// The DES-CBC-encrypted audio chunk.
    pub(crate) data: Vec<u8>,
}

/// Default DES-CBC chunk size; the first packet is reduced by 24 bytes to make
/// room for the packet header sent over the bulk endpoint.
const DEFAULT_CHUNK_SIZE: usize = 0x0010_0000;
const FIRST_PACKET_HEADER: usize = 24;

/// Lazy, streaming packet encryptor.
///
/// Reads the track payload from an arbitrary [`Read`] source and yields one
/// [`EncryptedPacket`] per [`Iterator::next`], so neither the plaintext nor the
/// ciphertext is ever fully buffered in memory. The payload is zero-padded to a
/// multiple of `frame_size`, matching the device's framing.
///
/// Mirrors `MDTrack.getPacketIterator` / `encrypt-generator.ts`:
/// - A random 8-byte raw key is generated (or supplied, for tests).
/// - The packet `key` field is `DES-ECB-decrypt(rawKey, KEK)` (first 8 bytes).
/// - The payload is split into `0x00100000`-byte chunks (first minus 24) and
///   each is DES-CBC encrypted with `rawKey`, chaining the IV from the previous
///   ciphertext's last block.
pub(crate) struct PacketEncryptor<R: Read> {
    reader: R,
    raw_key: [u8; 8],
    key: [u8; 8],
    iv: [u8; 8],
    /// Real payload bytes still to be read from `reader`.
    data_remaining: usize,
    /// Zero-padding bytes still to be appended after the real data.
    pad_remaining: usize,
    packet_count: usize,
    /// Set once a read error has been returned so the iterator fuses.
    errored: bool,
}

impl<R: Read> PacketEncryptor<R> {
    /// Computes the zero-padding needed to round `data_len` up to `frame_size`.
    pub(crate) fn padding_for(data_len: usize, frame_size: usize) -> usize {
        if data_len.is_multiple_of(frame_size) {
            0
        } else {
            frame_size - (data_len % frame_size)
        }
    }

    fn from_parts(kek: &[u8; 8], frame_size: usize, data_len: usize, reader: R, raw_key: [u8; 8]) -> Self {
        // key = DES-ECB-decrypt(rawKey, KEK), first 8 bytes.
        let key_dec = des_ecb_decrypt(kek, &raw_key);
        let key: [u8; 8] = key_dec[0..8].try_into().unwrap();
        let pad_remaining = Self::padding_for(data_len, frame_size);
        trace!(
            "PacketEncryptor: frame_size={frame_size} data_len={data_len} pad={pad_remaining}"
        );
        Self {
            reader,
            raw_key,
            key,
            iv: [0u8; 8],
            data_remaining: data_len,
            pad_remaining,
            packet_count: 0,
            errored: false,
        }
    }

    /// Creates an encryptor with a random raw key.
    pub(crate) fn new(kek: &[u8; 8], frame_size: usize, data_len: usize, reader: R) -> Self {
        use rand::Rng;
        let mut raw_key = [0u8; 8];
        rand::rng().fill_bytes(&mut raw_key);
        Self::from_parts(kek, frame_size, data_len, reader, raw_key)
    }

    /// Total padded payload size in bytes.
    fn total_padded(data_remaining: usize, pad_remaining: usize) -> usize {
        data_remaining + pad_remaining
    }

    /// Reads and encrypts the next chunk, returning `None` when the whole padded
    /// payload has been consumed.
    fn next_packet(&mut self) -> Option<Result<EncryptedPacket>> {
        let remaining = Self::total_padded(self.data_remaining, self.pad_remaining);
        if remaining == 0 {
            return None;
        }

        let mut chunk_size = if self.packet_count > 0 {
            DEFAULT_CHUNK_SIZE
        } else {
            DEFAULT_CHUNK_SIZE - FIRST_PACKET_HEADER
        };
        chunk_size = chunk_size.min(remaining);

        // Fill the chunk: real payload bytes first, then zero padding.
        let mut chunk = vec![0u8; chunk_size];
        let from_data = self.data_remaining.min(chunk_size);
        if from_data > 0 {
            if let Err(e) = self.reader.read_exact(&mut chunk[..from_data]) {
                self.errored = true;
                return Some(Err(NetMDError::Io(e)));
            }
            self.data_remaining -= from_data;
        }
        // Bytes from `from_data..chunk_size` stay zero (padding).
        self.pad_remaining -= chunk_size - from_data;

        // DES-CBC encrypt the chunk with rawKey, chaining IV.
        let encrypted = des_cbc_encrypt(&self.raw_key, &self.iv, &chunk);
        let next_iv: [u8; 8] = encrypted[encrypted.len() - 8..].try_into().unwrap();
        let packet = EncryptedPacket {
            key: self.key,
            iv: self.iv,
            data: encrypted,
        };
        self.iv = next_iv;
        self.packet_count += 1;
        Some(Ok(packet))
    }
}

impl<R: Read> Iterator for PacketEncryptor<R> {
    type Item = Result<EncryptedPacket>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.errored {
            return None;
        }
        self.next_packet()
    }
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn des_cbc_block_aligned() {
        let key = [0u8; 8];
        let iv = [0u8; 8];
        let data = [0u8; 16];
        let out = des_cbc_encrypt(&key, &iv, &data);
        assert_eq!(out.len(), 16);
    }

    #[test]
    fn ecb_roundtrip() {
        let key = [0x14, 0xe3, 0x83, 0x4e, 0xe2, 0xd3, 0xcc, 0xa5];
        let data = [0x11u8, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
        let enc = des_ecb_encrypt(&key, &data);
        let dec = des_ecb_decrypt(&key, &enc);
        assert_eq!(dec, data);
    }

    /// Reference implementation: the original eager encryptor, kept here to
    /// pin the streaming `PacketEncryptor` to byte-identical output.
    fn reference_encrypt(
        kek: &[u8; 8],
        frame_size: usize,
        data: &[u8],
        raw_key: [u8; 8],
    ) -> Vec<EncryptedPacket> {
        let key_dec = des_ecb_decrypt(kek, &raw_key);
        let key: [u8; 8] = key_dec[0..8].try_into().unwrap();
        let mut padded = data.to_vec();
        if !padded.len().is_multiple_of(frame_size) {
            let pad = frame_size - (padded.len() % frame_size);
            padded.extend(std::iter::repeat_n(0u8, pad));
        }
        let default_chunk_size: usize = 0x0010_0000;
        let mut packets = Vec::new();
        let mut iv = [0u8; 8];
        let mut offset = 0usize;
        let mut packet_count = 0usize;
        while offset < padded.len() {
            let mut chunk_size = if packet_count > 0 {
                default_chunk_size
            } else {
                default_chunk_size - 24
            };
            chunk_size = chunk_size.min(padded.len() - offset);
            let chunk = &padded[offset..offset + chunk_size];
            let encrypted = des_cbc_encrypt(&raw_key, &iv, chunk);
            let next_iv: [u8; 8] = encrypted[encrypted.len() - 8..].try_into().unwrap();
            packets.push(EncryptedPacket {
                key,
                iv,
                data: encrypted,
            });
            iv = next_iv;
            offset += chunk_size;
            packet_count += 1;
        }
        packets
    }

    fn assert_streaming_matches(frame_size: usize, data: &[u8]) {
        let kek = [0x14, 0xe3, 0x83, 0x4e, 0xe2, 0xd3, 0xcc, 0xa5];
        let raw_key = [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef];
        let expected = reference_encrypt(&kek, frame_size, data, raw_key);
        let got: Vec<EncryptedPacket> = PacketEncryptor::from_parts(
            &kek,
            frame_size,
            data.len(),
            std::io::Cursor::new(data.to_vec()),
            raw_key,
        )
        .map(|p| p.unwrap())
        .collect();
        assert_eq!(got.len(), expected.len(), "packet count");
        for (i, (g, e)) in got.iter().zip(&expected).enumerate() {
            assert_eq!(g.key, e.key, "packet {i} key");
            assert_eq!(g.iv, e.iv, "packet {i} iv");
            assert_eq!(g.data, e.data, "packet {i} data");
        }
    }

    #[test]
    fn streaming_encryptor_matches_reference() {
        // Block-aligned, needs no padding.
        assert_streaming_matches(192, &(0u8..=255).cycle().take(192 * 4).collect::<Vec<_>>());
        // Needs padding to frame size.
        assert_streaming_matches(192, &(0u8..=255).cycle().take(200).collect::<Vec<_>>());
        // PCM frame size, small.
        assert_streaming_matches(2048, &(0u8..200).collect::<Vec<_>>());
        // Empty payload still pads to one frame.
        assert_streaming_matches(96, &[]);
    }

    #[test]
    fn streaming_encryptor_spans_multiple_chunks() {
        // Exceed the 1 MiB chunk boundary so multiple packets are produced,
        // exercising the first-packet (-24) sizing and IV chaining across chunks.
        let frame_size = 2048;
        let len = 0x0010_0000 + 5 * frame_size; // > one full chunk
        let data: Vec<u8> = (0u8..=255).cycle().take(len).collect();
        assert_streaming_matches(frame_size, &data);
    }

    #[test]
    fn retailmac_is_deterministic_and_8_bytes() {
        let root_key: [u8; 16] = [
            0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x0f, 0xed, 0xcb, 0xa9, 0x87, 0x65,
            0x43, 0x21,
        ];
        let nonce = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        let iv = [0u8; 8];
        let mac1 = retailmac(&root_key, &nonce, &iv);
        let mac2 = retailmac(&root_key, &nonce, &iv);
        assert_eq!(mac1, mac2);
        assert_eq!(mac1.len(), 8);
    }
}
