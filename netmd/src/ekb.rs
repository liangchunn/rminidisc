//! EKB (Encrypted Key Block) data for the secure download pipeline.
//!
//! Ported from `netmd-js/src/netmd-ekb.ts`. The open-source EKB matches most
//! devices; the deck-specific `CorruptedDeckEKB` applies to MDS-JB980 / JE780 /
//! NT1 decks with a corrupted all-`0xff` leaf ID.

const SONY_VENDOR_ID: u16 = 0x054c;
const CORRUPTED_DECK_PRODUCT_ID: u16 = 0x0081;

/// Data needed to perform the key exchange with a device.
pub struct Ekb {
    /// The root key used to derive the session key via `retailmac`.
    pub root_key: [u8; 16],
    /// The EKB identifier (`sendKeyData` argument).
    pub ekb_id: u32,
    /// The key chain (each entry is 16 bytes).
    pub key_chain: Vec<[u8; 16]>,
    /// The key chain depth.
    pub depth: u8,
    /// The 24-byte EKB signature.
    pub signature: [u8; 24],
}

/// Returns the open-source EKB. Mirrors `EKBOpenSource` (`netmd-ekb.ts:11`).
///
/// `getEKBForDevice` in the JS reference selects this as the catch-all fallback.
pub fn open_source_ekb() -> Ekb {
    Ekb {
        root_key: [
            0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x0f, 0xed, 0xcb, 0xa9, 0x87, 0x65,
            0x43, 0x21,
        ],
        ekb_id: 0x2642_2642,
        key_chain: vec![
            [
                0x25, 0x45, 0x06, 0x4d, 0xea, 0xca, 0x14, 0xf9, 0x96, 0xbd, 0xc8, 0xa4, 0x06, 0xc2,
                0x2b, 0x81,
            ],
            [
                0xfb, 0x60, 0xbd, 0xdd, 0x0d, 0xbc, 0xab, 0x84, 0x8a, 0x00, 0x5e, 0x03, 0x19, 0x4d,
                0x3e, 0xda,
            ],
        ],
        depth: 9,
        signature: [
            0x8f, 0x2b, 0xc3, 0x52, 0xe8, 0x6c, 0x5e, 0xd3, 0x06, 0xdc, 0xae, 0x18, 0xd2, 0xf3,
            0x8c, 0x7f, 0x89, 0xb5, 0xe1, 0x85, 0x55, 0xa1, 0x05, 0xea,
        ],
    }
}

/// Returns the corrupted-deck EKB. Mirrors `CorruptedDeckEKB` (`netmd-ekb.ts`).
pub fn corrupted_deck_ekb() -> Ekb {
    Ekb {
        root_key: [
            0x57, 0x4d, 0x44, 0x50, 0x57, 0x4d, 0x44, 0x50, 0x4d, 0x69, 0x6e, 0x69, 0x44, 0x69,
            0x73, 0x63,
        ],
        ekb_id: 0x1337_1337,
        key_chain: vec![[
            0xb1, 0xd4, 0xaf, 0xfa, 0x80, 0xa0, 0xc9, 0x03, 0xc2, 0x58, 0x4b, 0x1b, 0x44, 0xaf,
            0xc4, 0xa6,
        ]],
        depth: 9,
        signature: [
            0x6c, 0x2b, 0xc2, 0x8c, 0x45, 0x2b, 0x54, 0xf1, 0xc3, 0x59, 0x72, 0x3b, 0xe3, 0x19,
            0x1f, 0x55, 0x17, 0x25, 0x64, 0x0e, 0x65, 0x8c, 0x81, 0x0b,
        ],
    }
}

fn is_corrupted_deck_ekb_match(leaf_id: &[u8], vendor: u16, product: u16) -> bool {
    !leaf_id.is_empty()
        && leaf_id.iter().all(|byte| *byte == 0xff)
        && vendor == SONY_VENDOR_ID
        && product == CORRUPTED_DECK_PRODUCT_ID
}

/// Selects the appropriate EKB for the device. Mirrors `getEKBForDevice`
/// (`netmd-ekb.ts`) by checking the deck-specific EKB before the catch-all
/// open-source EKB.
pub fn get_ekb_for_device(leaf_id: &[u8], vendor: u16, product: u16) -> Ekb {
    if is_corrupted_deck_ekb_match(leaf_id, vendor, product) {
        corrupted_deck_ekb()
    } else {
        open_source_ekb()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_corrupted_deck_ekb_for_sony_0081_with_ff_leaf_id() {
        let ekb = get_ekb_for_device(&[0xff; 8], SONY_VENDOR_ID, CORRUPTED_DECK_PRODUCT_ID);

        assert_eq!(ekb.ekb_id, 0x1337_1337);
        assert_eq!(ekb.root_key, *b"WMDPWMDPMiniDisc");
        assert_eq!(ekb.key_chain.len(), 1);
        assert_eq!(ekb.depth, 9);
    }

    #[test]
    fn falls_back_to_open_source_ekb_for_normal_leaf_id() {
        let ekb = get_ekb_for_device(&[0x01, 0x02, 0x03], SONY_VENDOR_ID, CORRUPTED_DECK_PRODUCT_ID);

        assert_eq!(ekb.ekb_id, 0x2642_2642);
        assert_eq!(ekb.key_chain.len(), 2);
    }

    #[test]
    fn falls_back_to_open_source_ekb_for_other_devices_with_ff_leaf_id() {
        let ekb = get_ekb_for_device(&[0xff; 8], SONY_VENDOR_ID, 0x0084);

        assert_eq!(ekb.ekb_id, 0x2642_2642);
    }
}
