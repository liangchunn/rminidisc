use std::thread::sleep;
use std::time::Duration;

use log::{debug, info, trace};
use rusb::{DeviceHandle, UsbContext};

use crate::{
    crypto::{self, EncryptedPacket},
    query::QueryBuilder,
    status::get_operating_status,
    transport::{read_reply_after_bulk, read_reply_length, send_query, send_query_ext, write_bulk},
    types::OperatingStatus,
    util::parse_u16,
};

/// Common command prefix for all secure-session commands.
///
/// The leading `00` is the status/command byte that `NetMDInterface.sendCommand`
/// (`netmd.ts:226`) prepends to every query before the control write. The
/// existing non-secure commands in this crate spell that byte out explicitly in
/// their hex strings (e.g. `"00 1806 ..."`); the secure commands include it here
/// so the device echoes back `1800 080046 ...` with the status replacing the
/// leading `00`, matching the reply scan templates.
const SECURE_PREFIX: &str = "00 1800 080046 f0030103";

/// Acquires the device lock (`ff 010c ...`). Mirrors `NetMDInterface.acquire`.
pub fn acquire<T: UsbContext>(handle: &DeviceHandle<T>) -> anyhow::Result<()> {
    debug!("acquire");
    let reply = send_query(handle, "00 ff 010c ffff ffff ffff ffff ffff ffff")?;
    reply.scan("%? ff 010c ffff ffff ffff ffff ffff ffff")?;
    Ok(())
}

/// Releases the device lock (`ff 0100 ...`). Mirrors `NetMDInterface.release`.
pub fn release<T: UsbContext>(handle: &DeviceHandle<T>) -> anyhow::Result<()> {
    debug!("release");
    let reply = send_query(handle, "00 ff 0100 ffff ffff ffff ffff ffff ffff")?;
    reply.scan("%? ff 0100 ffff ffff ffff ffff ffff ffff")?;
    Ok(())
}

/// Enters a secure session. Mirrors `enterSecureSession` (`netmd-interface.ts:729`).
pub fn enter_secure_session<T: UsbContext>(handle: &DeviceHandle<T>) -> anyhow::Result<()> {
    debug!("enter secure session");
    let query = QueryBuilder::new().raw(SECURE_PREFIX)?.raw("80 ff")?;
    let reply = send_query(handle, query)?;
    reply.scan("%? 1800 080046 f0030103 80 00")?;
    Ok(())
}

/// Leaves a secure session. Mirrors `leaveSecureSession` (`netmd-interface.ts:735`).
pub fn leave_secure_session<T: UsbContext>(handle: &DeviceHandle<T>) -> anyhow::Result<()> {
    debug!("leave secure session");
    let query = QueryBuilder::new().raw(SECURE_PREFIX)?.raw("81 ff")?;
    let reply = send_query(handle, query)?;
    reply.scan("%? 1800 080046 f0030103 81 00")?;
    Ok(())
}

/// Reads the device leaf ID. Mirrors `getLeafID` (`netmd-interface.ts:747`).
pub fn get_leaf_id<T: UsbContext>(handle: &DeviceHandle<T>) -> anyhow::Result<Vec<u8>> {
    debug!("get leaf id");
    let query = QueryBuilder::new().raw(SECURE_PREFIX)?.raw("11 ff")?;
    let reply = send_query(handle, query)?;
    let data = reply.scan("%? 1800 080046 f0030103 11 00 %*")?;
    Ok(data[0].to_vec())
}

/// Sends the EKB key data. Mirrors `sendKeyData` (`netmd-interface.ts:754`).
pub fn send_key_data<T: UsbContext>(
    handle: &DeviceHandle<T>,
    ekb_id: u32,
    key_chain: &[[u8; 16]],
    depth: u8,
    signature: &[u8; 24],
) -> anyhow::Result<()> {
    debug!("send key data (ekb_id=0x{ekb_id:08x} depth={depth})");
    if !(1..=63).contains(&depth) {
        anyhow::bail!("invalid EKB depth: {depth}");
    }
    let chain_len = key_chain.len() as u32;
    let databytes = 16 + 16 * chain_len + 24;
    let mut chain_bytes = Vec::with_capacity(16 * key_chain.len());
    for k in key_chain {
        chain_bytes.extend_from_slice(k);
    }

    // formatQuery('... 12 ff %w 0000 %w %d %d %d 00000000 %* %*',
    //   databytes, databytes, chainlen, depth, ekbid, keychains, ekbsignature)
    let query = QueryBuilder::new()
        .raw(SECURE_PREFIX)?
        .raw("12 ff")?
        .u16(databytes as u16)
        .raw("0000")?
        .u16(databytes as u16)
        .u32(chain_len)
        .u32(depth as u32)
        .u32(ekb_id)
        .raw("00000000")?
        .bytes(&chain_bytes)
        .bytes(signature);
    let reply = send_query(handle, query)?;
    reply.scan("%? 1800 080046 f0030103 12 01 %?%? %?%?%?%?")?;
    Ok(())
}

/// Performs session key exchange. Mirrors `sessionKeyExchange` (`netmd-interface.ts:783`).
/// Returns the 8-byte device nonce.
pub fn session_key_exchange<T: UsbContext>(
    handle: &DeviceHandle<T>,
    host_nonce: &[u8; 8],
) -> anyhow::Result<[u8; 8]> {
    debug!("session key exchange");
    let query = QueryBuilder::new()
        .raw(SECURE_PREFIX)?
        .raw("20 ff 000000")?
        .bytes(host_nonce);
    let reply = send_query(handle, query)?;
    // '20 %?' instead of '20 00' (Panasonic fix); %# = consume-to-end.
    let data = reply.scan("%? 1800 080046 f0030103 20 %? 000000 %#")?;
    let dev_nonce: [u8; 8] = data[0]
        .try_into()
        .map_err(|_| anyhow::anyhow!("device nonce wrong length"))?;
    Ok(dev_nonce)
}

/// Forgets the session key. Mirrors `sessionKeyForget` (`netmd-interface.ts:792`).
pub fn session_key_forget<T: UsbContext>(handle: &DeviceHandle<T>) -> anyhow::Result<()> {
    debug!("session key forget");
    let query = QueryBuilder::new()
        .raw(SECURE_PREFIX)?
        .raw("21 ff 000000")?;
    let reply = send_query(handle, query)?;
    reply.scan("%? 1800 080046 f0030103 21 00 000000")?;
    Ok(())
}

/// Sets up a download. Mirrors `setupDownload` (`netmd-interface.ts:798`).
///
/// Encrypts `[1,1,1,1] + content_id(20) + kek(8)` with DES-CBC (NoPadding, zero
/// IV) under the session key, then sends it.
pub fn setup_download<T: UsbContext>(
    handle: &DeviceHandle<T>,
    content_id: &[u8; 20],
    kek: &[u8; 8],
    session_key: &[u8; 8],
) -> anyhow::Result<()> {
    debug!("setup download");
    let mut message = Vec::with_capacity(32);
    message.extend_from_slice(&[1, 1, 1, 1]);
    message.extend_from_slice(content_id);
    message.extend_from_slice(kek);
    let encrypted = crypto::des_cbc_encrypt(session_key, &[0u8; 8], &message);

    let query = QueryBuilder::new()
        .raw(SECURE_PREFIX)?
        .raw("22 ff 0000")?
        .bytes(&encrypted);
    let reply = send_query(handle, query)?;
    reply.scan("%? 1800 080046 f0030103 22 00 0000")?;
    Ok(())
}

/// Disables new-track copy protection. Mirrors `disableNewTrackProtection`
/// (`netmd-interface.ts:723`).
pub fn disable_new_track_protection<T: UsbContext>(
    handle: &DeviceHandle<T>,
    val: u16,
) -> anyhow::Result<()> {
    debug!("disable new track protection ({val})");
    let query = QueryBuilder::new()
        .raw(SECURE_PREFIX)?
        .raw("2b ff")?
        .u16(val);
    let reply = send_query(handle, query)?;
    reply.scan("%? 1800 080046 f0030103 2b 00 %?%?")?;
    Ok(())
}

/// Commits a track after upload. Mirrors `commitTrack` (`netmd-interface.ts:822`).
///
/// Authentication = DES-ECB encrypt of 8 zero bytes under the session key.
pub fn commit_track<T: UsbContext>(
    handle: &DeviceHandle<T>,
    track: u16,
    session_key: &[u8; 8],
) -> anyhow::Result<()> {
    debug!("commit track #{track}");
    let authentication = crypto::des_ecb_encrypt(session_key, &[0u8; 8]);
    let query = QueryBuilder::new()
        .raw(SECURE_PREFIX)?
        .raw("48 ff 00 1001")?
        .u16(track)
        .bytes(&authentication);
    let reply = send_query(handle, query)?;
    reply.scan("%? 1800 080046 f0030103 48 00 00 1001 %?%?")?;
    Ok(())
}

/// Terminates the secure session lifecycle. Mirrors `terminate` (`netmd-interface.ts:909`).
pub fn terminate<T: UsbContext>(handle: &DeviceHandle<T>) -> anyhow::Result<()> {
    debug!("terminate");
    let query = QueryBuilder::new().raw(SECURE_PREFIX)?.raw("2a ff00")?;
    // JS does not scan the reply.
    send_query(handle, query)?;
    Ok(())
}

/// Sends an encrypted track over the bulk endpoint. Mirrors `sendTrack`
/// (`netmd-interface.ts:839`).
///
/// Returns `(track_number, uuid_hex, content_id_hex)`.
#[allow(clippy::too_many_arguments)]
pub fn send_track<T: UsbContext>(
    handle: &DeviceHandle<T>,
    wireformat: u8,
    discformat: u8,
    frames: u32,
    pkt_size: u32,
    packets: &[EncryptedPacket],
    session_key: &[u8; 8],
    mut progress: Option<&mut dyn FnMut(u64, u64)>,
) -> anyhow::Result<(u16, String, String)> {
    debug!("send track (wf=0x{wireformat:02x} df=0x{discformat:02x} frames={frames})");
    info!("sending track: {} packets, {} total bytes", packets.len(), pkt_size + 24);
    // The sharps are slow...
    sleep(Duration::from_millis(200));

    let total_bytes: u64 = pkt_size as u64 + 24;

    // 28 ff 000100 1001 ffff 00 %b %b %d %d
    let query = QueryBuilder::new()
        .raw(SECURE_PREFIX)?
        .raw("28 ff 000100 1001 ffff 00")?
        .u8(wireformat)
        .u8(discformat)
        .u32(frames)
        .u32(total_bytes as u32);
    // Accept the interim response.
    let reply = send_query_ext(handle, query, true)?;
    reply.scan("%? 1800 080046 f0030103 28 00 000100 1001 %?%? 00 %*")?;

    sleep(Duration::from_millis(200));

    let mut written_bytes: u64 = 0;
    for (i, packet) in packets.iter().enumerate() {
        if let Some(cb) = progress.as_deref_mut() {
            cb(written_bytes, total_bytes);
        }
        let binpack = if i == 0 {
            // First packet header: 4 zero bytes, then the packed length as a
            // big-endian u32 (`sendTrack` reverses the LE buffer on LE hosts -
            // netmd-interface.ts:871), then key, iv, data.
            let mut buf = Vec::with_capacity(24 + packet.data.len());
            buf.extend_from_slice(&[0, 0, 0, 0]);
            buf.extend_from_slice(&pkt_size.to_be_bytes());
            buf.extend_from_slice(&packet.key);
            buf.extend_from_slice(&packet.iv);
            buf.extend_from_slice(&packet.data);
            buf
        } else {
            packet.data.clone()
        };
        write_bulk(handle, &binpack)?;
        written_bytes += packet.data.len() as u64;
    }
    if let Some(cb) = progress {
        cb(written_bytes, total_bytes);
    }

    // Read the final reply. The device commits the track before replying, which
    // can take several seconds, so poll the reply-length register tolerating
    // USB timeouts rather than erroring on the first one.
    let final_reply = read_reply_after_bulk(handle)?;
    // Refresh the reply-length register (JS calls getReplyLength again).
    let _ = read_reply_length(handle);

    let data = final_reply
        .scan("%? 1800 080046 f0030103 28 00 000100 1001 %w 00 %?%? %?%?%?%? %?%?%?%? %*")?;
    let track_number = parse_u16(data[0])?;
    let encrypted_reply = data[1];

    // Decrypt the reply with DES-CBC (zero IV) under the session key.
    let decrypted = if encrypted_reply.len() % 8 == 0 && !encrypted_reply.is_empty() {
        crypto::des_cbc_decrypt(session_key, &[0u8; 8], encrypted_reply)
    } else {
        encrypted_reply.to_vec()
    };
    let uuid = hex_string(decrypted.get(0..8).unwrap_or(&[]));
    let content_id = hex_string(decrypted.get(12..32).unwrap_or(&[]));

    Ok((track_number, uuid, content_id))
}

fn hex_string(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Waits until the device is ready/blank for a download, then acquires it and
/// disables new-track protection. Mirrors `prepareDownload` (`netmd-commands.ts:444`).
pub fn prepare_download<T: UsbContext>(handle: &DeviceHandle<T>) -> anyhow::Result<()> {
    debug!("prepare download");
    // Wait for the device to be ready or for a blank disc.
    for i in 0..50 {
        match get_operating_status(handle) {
            Ok(OperatingStatus::Ready) | Ok(OperatingStatus::BlankDisc) => {
                info!("device ready for download (poll #{i})");
                break;
            }
            Ok(status) => {
                trace!("  poll #{i}: device status={status:?}");
            }
            _ => {
                trace!("  poll #{i}: no status yet");
            }
        }
        sleep(Duration::from_millis(200));
    }
    // Best-effort: forget any prior session.
    let _ = session_key_forget(handle);
    let _ = leave_secure_session(handle);

    acquire(handle)?;
    // On Sharp devices this doesn't work; ignore errors.
    let _ = disable_new_track_protection(handle, 1);
    Ok(())
}
