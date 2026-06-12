//! EKB (Encrypted Key Block) data for the secure download pipeline.
//!
//! Ported from `netmd-js/src/netmd-ekb.ts`. Only `EKBOpenSource` is ported — it
//! matches every device (`doesEKBMatchDevice` always returns true) and is what
//! the MZ-N505 uses. The deck-specific `CorruptedDeckEKB` is not ported (it only
//! applies to MDS-JB980 / JE780 / NT1 decks with a corrupted leaf ID).

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
/// `getEKBForDevice` in the JS reference selects between this and the deck EKB;
/// since the deck EKB is not ported and the open-source one matches all devices,
/// this is the only selection needed for the supported hardware.
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

/// Selects the appropriate EKB for the device. Currently always the open-source
/// EKB. Mirrors `getEKBForDevice` (`netmd-ekb.ts:83`) minus the deck branch.
pub fn get_ekb_for_device(_leaf_id: &[u8], _vendor: u16, _product: u16) -> Ekb {
    open_source_ekb()
}
