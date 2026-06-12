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

use cipher::generic_array::GenericArray;
use cipher::{BlockDecryptMut, BlockEncryptMut, KeyInit, KeyIvInit};
use des::{Des, TdesEde2};

type DesCbcEnc = cbc::Encryptor<Des>;
type DesCbcDec = cbc::Decryptor<Des>;
type DesEcbEnc = ecb::Encryptor<Des>;
type DesEcbDec = ecb::Decryptor<Des>;
type TdesCbcEnc = cbc::Encryptor<TdesEde2>;

/// DES-CBC encrypt without padding. `data.len()` must be a multiple of 8.
pub fn des_cbc_encrypt(key: &[u8; 8], iv: &[u8; 8], data: &[u8]) -> Vec<u8> {
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
pub fn des_cbc_decrypt(key: &[u8; 8], iv: &[u8; 8], data: &[u8]) -> Vec<u8> {
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
pub fn des_ecb_encrypt(key: &[u8; 8], data: &[u8]) -> Vec<u8> {
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
pub fn des_ecb_decrypt(key: &[u8; 8], data: &[u8]) -> Vec<u8> {
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
pub fn tdes_cbc_encrypt(key: &[u8; 16], iv: &[u8; 8], data: &[u8]) -> Vec<u8> {
    assert_eq!(data.len() % 8, 0, "tdes_cbc_encrypt: data not block-aligned");
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
pub fn retailmac(key: &[u8; 16], value: &[u8], iv: &[u8; 8]) -> [u8; 8] {
    assert!(value.len() >= 8, "retailmac: value too short");
    assert_eq!(value.len() % 8, 0, "retailmac: value not block-aligned");

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
pub struct EncryptedPacket {
    /// The wrapped data key (KEK-decrypted random key), 8 bytes. Sent in the
    /// first packet header only.
    pub key: [u8; 8],
    /// The IV for this packet (8 bytes). Sent in the first packet header only.
    pub iv: [u8; 8],
    /// The DES-CBC-encrypted audio chunk.
    pub data: Vec<u8>,
}

/// Encrypts the track payload into packets.
///
/// Mirrors `MDTrack.getPacketIterator` / `encrypt-generator.ts`:
/// - A random 8-byte raw key is generated.
/// - The packet `key` field is `DES-ECB-decrypt(rawKey, KEK)` (first 8 bytes).
/// - Audio data is split into chunks (`0x00100000`, first chunk minus 24 bytes
///   to leave room for the packet header) and each chunk is DES-CBC encrypted
///   with `rawKey`, chaining the IV from the previous ciphertext's last block.
/// - `data` is padded to a multiple of `frame_size` with zeros first.
pub fn encrypt_packets(kek: &[u8; 8], frame_size: usize, data: &[u8]) -> Vec<EncryptedPacket> {
    use rand::RngCore;

    let mut raw_key = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut raw_key);

    // key = DES-ECB-decrypt(rawKey, KEK), first 8 bytes.
    let key_dec = des_ecb_decrypt(kek, &raw_key);
    let key: [u8; 8] = key_dec[0..8].try_into().unwrap();

    // Pad data to frame size.
    let mut padded = data.to_vec();
    if padded.len() % frame_size != 0 {
        let pad = frame_size - (padded.len() % frame_size);
        padded.extend(std::iter::repeat(0u8).take(pad));
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
        // DES-CBC encrypt the chunk with rawKey, chaining IV.
        let encrypted = des_cbc_encrypt(&raw_key, &iv, chunk);

        // Next IV = last 8 bytes of this ciphertext.
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

    #[test]
    fn retailmac_is_deterministic_and_8_bytes() {
        let root_key: [u8; 16] = [
            0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x0f, 0xed, 0xcb, 0xa9, 0x87, 0x65,
            0x43, 0x21,
        ];
        let nonce = [
            1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
        ];
        let iv = [0u8; 8];
        let mac1 = retailmac(&root_key, &nonce, &iv);
        let mac2 = retailmac(&root_key, &nonce, &iv);
        assert_eq!(mac1, mac2);
        assert_eq!(mac1.len(), 8);
    }
}
