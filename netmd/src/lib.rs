//! NetMD device protocol library.
//!
//! Provides the command functions for talking to a Sony NetMD device over USB,
//! ported from `netmd-js`. Device discovery/lifecycle (`open_device`,
//! `close_device`) and all per-command functions live here; binaries should be
//! thin wrappers over this crate.

// Many command functions and protocol enums are part of the ported NetMD API
// surface but are not yet wired into a CLI. Allow them to exist without
// dead-code warnings while the port is in progress.
#![allow(dead_code)]

use std::thread::sleep;
use std::time::Duration;

use log::{debug, trace};
use rusb::{request_type, DeviceHandle, GlobalContext, UsbContext};

use crate::{
    descriptor::{Descriptor, DescriptorAction, DescriptorCommand},
    error::NetMDError,
    query::Query,
    scan::scan,
    types::{
        ProtocolReply, ReadRequestData, ReadRequestHeader, INTERIM_RETRY_INTERVAL_MS,
        MAX_INTERIM_ATTEMPTS, READ_REPLY_POLL_INTERVAL_MS, USB_TIMEOUT_MILLIS,
    },
    util::{
        encode_to_sjis, get_length_after_sjis_encode, parse_bcd_u16, parse_bcd_u8, parse_string,
        parse_u16, parse_u8,
    },
};

pub mod descriptor;
pub mod error;
pub mod query;
pub mod scan;
pub mod types;
pub mod util;

// --- Public re-exports for ergonomic access from downstream crates. ---
// (The internal `use crate::descriptor::{...}` above already brings the
// descriptor types into this module's namespace; the modules are also public,
// so downstream crates can reach everything via `netmd::descriptor::...` etc.)
pub use error::NetMDError as Error;
pub use types::{
    Channels, ChannelCount, DiscFlag, DiscFormat, Encoding, NetMDLevel, ProtocolReply as Status,
    TrackFlag, Wireformat, FRAME_SIZE,
};
pub use util::{format_time_from_frames, time_to_frames};

/// Sony USB vendor ID.
pub const SONY_VENDOR_ID: u16 = 0x054c;
/// Product ID of the Sony MZ-N505 (the currently supported device).
pub const MZ_N505_PRODUCT_ID: u16 = 0x0084;

/// Opens the first connected supported NetMD device and claims its interface.
///
/// Mirrors the discovery/open logic previously inlined in the runner. Filters
/// for the Sony VID/PID, opens the handle, logs the manufacturer string, and
/// claims interface 0. All device-touching logic lives in this crate.
pub fn open_device() -> anyhow::Result<DeviceHandle<GlobalContext>> {
    let devices = rusb::devices()?
        .iter()
        .filter(|device| {
            device
                .device_descriptor()
                .map(|d| d.vendor_id() == SONY_VENDOR_ID && d.product_id() == MZ_N505_PRODUCT_ID)
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    let device = devices.first().ok_or_else(|| anyhow::anyhow!("cannot find device"))?;
    let device_desc = device.device_descriptor()?;
    let handle = device.open()?;

    if let Ok(langs) = handle.read_languages(Duration::from_secs(5)) {
        let manufacturer = langs
            .iter()
            .filter_map(|lang| {
                handle
                    .read_manufacturer_string(*lang, &device_desc, Duration::from_secs(5))
                    .ok()
            })
            .collect::<Vec<_>>();
        debug!(
            "opened {:04x}:{:04x} ({})",
            device_desc.vendor_id(),
            device_desc.product_id(),
            manufacturer.join(", ")
        );
    }

    handle.claim_interface(0)?;
    Ok(handle)
}

/// Releases the claimed interface. Mirrors the runner's previous teardown.
pub fn close_device<T: UsbContext>(handle: &DeviceHandle<T>) -> anyhow::Result<()> {
    handle.release_interface(0)?;
    Ok(())
}

// TODO: to return concrete `.try_into()`` errors, use the return type:
// TODO:    Result<ReadRequestData, M::Error>
// TODO: and remove
// TODO:    anyhow::Error: From<M::Error>
pub fn send_query<T, M>(handle: &DeviceHandle<T>, message: M) -> anyhow::Result<ReadRequestData>
where
    T: UsbContext,
    M: TryInto<Query>,
    anyhow::Error: From<M::Error>,
{
    send_query_ext(handle, message, false)
}

/// Sends a command and reads the reply, performing protocol status checking.
///
/// Mirrors `NetMDInterface.sendQuery` + `readReply`:
/// - The command is sent once via control transfer (request `0x80`).
/// - The reply is read; if the status byte is interim (`0x0f`) and
///   `accept_interim` is false, the read is retried with exponential backoff.
/// - `0x08` maps to `NotImplemented`, `0x0a` to `Rejected`.
pub fn send_query_ext<T, M>(
    handle: &DeviceHandle<T>,
    message: M,
    accept_interim: bool,
) -> anyhow::Result<ReadRequestData>
where
    T: UsbContext,
    M: TryInto<Query>,
    anyhow::Error: From<M::Error>,
{
    let query: Query = message.try_into()?;
    trace!("  TX → {:02x?}", query.0);
    handle.write_control(
        request_type(
            rusb::Direction::Out,
            rusb::RequestType::Vendor,
            rusb::Recipient::Interface,
        ),
        0x80,
        0,
        0,
        &query.0,
        Duration::from_millis(USB_TIMEOUT_MILLIS),
    )?;

    let reply = read_reply_checked(handle, accept_interim)?;

    Ok(reply)
}

/// Reads a reply, checking the protocol status byte and retrying on interim.
///
/// The status byte is the first byte of the reply payload. On success the
/// status byte is left in place (callers strip it via scan `%?` templates).
pub fn read_reply_checked<T: UsbContext>(
    handle: &DeviceHandle<T>,
    accept_interim: bool,
) -> Result<ReadRequestData, NetMDError> {
    let mut attempt: u32 = 0;
    while attempt < MAX_INTERIM_ATTEMPTS {
        let data = read_reply(handle)?;
        let status: ProtocolReply = data
            .0
            .first()
            .copied()
            .ok_or(NetMDError::UnknownStatus(0))?
            .into();

        match status {
            ProtocolReply::NotImplemented => return Err(NetMDError::NotImplemented),
            ProtocolReply::Rejected => {
                let hex = data
                    .0
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>();
                return Err(NetMDError::Rejected(hex));
            }
            ProtocolReply::Interim if !accept_interim => {
                // Backoff: interval * (2^attempt - 1)
                let factor = (1u64 << attempt) - 1;
                sleep(Duration::from_millis(INTERIM_RETRY_INTERVAL_MS * factor));
                attempt += 1;
                continue;
            }
            ProtocolReply::Accepted
            | ProtocolReply::Implemented
            | ProtocolReply::Changed
            | ProtocolReply::Interim => {
                return Ok(data);
            }
            other => return Err(NetMDError::UnknownStatus(other as u8)),
        }
    }
    Err(NetMDError::MaxInterimAttempts)
}

/// Reads the raw disc title (chunked). Mirrors `NetMDInterface._getDiscTitle`.
///
/// Opens the audioContents + discTitle descriptors, reads all chunks, then
/// closes them. `w_char` selects the full-width (wchar) title table.
pub fn get_disk_title<T: UsbContext>(handle: &DeviceHandle<T>, w_char: bool) -> anyhow::Result<String> {
    change_descriptor_state(
        handle,
        Descriptor::AudioContentsTd,
        DescriptorAction::OpenRead,
    )?;
    change_descriptor_state(handle, Descriptor::DiskTitleTd, DescriptorAction::OpenRead)?;

    let mut done = 0;
    let mut remaining = 0;
    let mut total = 1;
    let mut chunk_size: u16;

    let mut sink = vec![];

    let w_char: u8 = if w_char { 1 } else { 0 };

    debug!("get disk title");

    while done < total {
        let query = format!(
            "00 1806 02201801 00{:02x} 3000 0a00 ff00 {:04x}{:04x}",
            w_char, remaining, done
        );
        let reply = send_query(handle, query)?;

        if remaining == 0 {
            let data = scan(
                "%? 1806 02201801 00%? 3000 0a00 1000 %w0000 %?%?000a %w %*",
                &reply.0,
            )?;

            if let [cz, t, d] = &data[..] {
                chunk_size = parse_u16(cz)?;
                total = parse_u16(t)?;
                sink.push(parse_string(d)?);
            } else {
                unreachable!()
            }
            chunk_size -= 6;
        } else {
            let data = reply.scan("%? 1806 02201801 00%? 3000 0a00 1000 %w%?%? %*")?;
            if let [cz, d] = &data[..] {
                chunk_size = parse_u16(cz)?;
                sink.push(parse_string(d)?);
            } else {
                unreachable!()
            }
        }
        done += chunk_size;
        remaining = total - done;
    }

    change_descriptor_state(handle, Descriptor::DiskTitleTd, DescriptorAction::Close)?;
    change_descriptor_state(handle, Descriptor::AudioContentsTd, DescriptorAction::Close)?;

    Ok(sink.join(""))
}

/// Returns the user-facing disc title, stripping the trailing group structure.
///
/// Mirrors `NetMDInterface.getDiscTitle`. If the raw title ends with the group
/// delimiter (`//` or full-width `／／`), the leading `0;`/`０；` title cell is
/// extracted; otherwise the title is cleared.
pub fn get_disc_title<T: UsbContext>(handle: &DeviceHandle<T>, w_char: bool) -> anyhow::Result<String> {
    let mut title = get_disk_title(handle, w_char)?;

    let delim = if w_char { "／／" } else { "//" };
    let title_marker = if w_char { "０；" } else { "0;" };

    if title.ends_with(delim) {
        let first_entry = title.split(delim).next().unwrap_or("");
        if let Some(stripped) = first_entry.strip_prefix(title_marker) {
            title = stripped.to_string();
        } else {
            title = String::new();
        }
    }
    Ok(title)
}

/// Reads a track's title. Mirrors `NetMDInterface.getTrackTitle`.
///
/// `track` is 0-based. `w_char` selects the full-width title (UTOC4) vs the
/// half-width title (UTOC1).
pub fn get_track_title<T: UsbContext>(
    handle: &DeviceHandle<T>,
    track: u16,
    w_char: bool,
) -> anyhow::Result<String> {
    let wchar_value: u8 = if w_char { 3 } else { 2 };
    let descriptor = if w_char {
        Descriptor::AudioUtoc4Td
    } else {
        Descriptor::AudioUtoc1Td
    };
    change_descriptor_state(handle, descriptor, DescriptorAction::OpenRead)?;
    let query = format!(
        "00 1806 022018{:02x} {:04x} 3000 0a00 ff00 00000000",
        wchar_value, track
    );
    let reply = send_query(handle, query)?;
    let data = reply.scan("%? 1806 022018%? %?%? %?%? %?%? 1000 00%?0000 00%?000a %x")?;
    let title = parse_string(data[0])?;
    change_descriptor_state(handle, descriptor, DescriptorAction::Close)?;
    Ok(title)
}

pub fn get_track_count<T: UsbContext>(handle: &DeviceHandle<T>) -> anyhow::Result<u8> {
    debug!("get track count");
    change_descriptor_state(
        handle,
        Descriptor::AudioContentsTd,
        DescriptorAction::OpenRead,
    )?;
    let reply = send_query(handle, "00 1806 02101001 3000 1000 ff00 00000000")?;

    let data = reply.scan("%? 1806 02101001 %?%? %?%? 1000 00%?0000 0006 0010000200%b")?;

    let track_count = if let [tc] = &data[..] {
        parse_u8(tc)?
    } else {
        unreachable!()
    };
    change_descriptor_state(
        handle,
        Descriptor::AudioContentsTd,
        DescriptorAction::Close,
    )?;
    Ok(track_count)
}

/// Reads the reply length header (request `0x01`). The third byte holds the
/// payload length. Mirrors `NetMD.getReplyLength`.
pub fn read_reply_length<T: UsbContext>(
    handle: &DeviceHandle<T>,
) -> Result<ReadRequestHeader, rusb::Error> {
    let mut reply_header = ReadRequestHeader::new();
    handle.read_control(
        request_type(
            rusb::Direction::In,
            rusb::RequestType::Vendor,
            rusb::Recipient::Interface,
        ),
        0x01,
        0,
        0,
        &mut reply_header.0,
        Duration::from_millis(USB_TIMEOUT_MILLIS),
    )?;
    trace!("  RX ← {:02x?}", reply_header.0);
    Ok(reply_header)
}

/// Sets the disc title. Mirrors `NetMDInterface.setDiscTitle` (standard branch).
///
/// NOTE: title sanitization (`sanitizeHalfWidthTitle`/`sanitizeFullWidthTitle`)
/// is NOT applied — the title is encoded to SHIFT_JIS as-is. See UNPORTED.md.
/// The Sharp (vendor 0x04dd) descriptor variant is also not handled.
pub fn set_disc_title<T: UsbContext>(
    handle: &DeviceHandle<T>,
    title: &str,
    w_char: bool,
) -> anyhow::Result<()> {
    debug!("set disc title: {title:?} (wchar={w_char})");
    let current_title = get_disk_title(handle, w_char)?;
    if current_title == title {
        // Setting the same title causes problems with the LAM.
        return Ok(());
    }
    let old_len = get_length_after_sjis_encode(&current_title)?;
    let new_len = get_length_after_sjis_encode(title)?;
    let wchar_value: u8 = if w_char { 1 } else { 0 };
    let sjis = encode_to_sjis(title)?;
    let sjis_hex = sjis.iter().map(|b| format!("{b:02x}")).collect::<String>();

    change_descriptor_state(handle, Descriptor::DiskTitleTd, DescriptorAction::Close)?;
    change_descriptor_state(handle, Descriptor::DiskTitleTd, DescriptorAction::OpenWrite)?;

    let query = format!(
        "00 1807 02201801 00{:02x} 3000 0a00 5000 {:04x} 0000 {:04x} {}",
        wchar_value, new_len, old_len, sjis_hex
    );
    let reply = send_query(handle, query)?;
    reply.scan("%? 1807 02201801 00%? 3000 0a00 5000 %?%? 0000 %?%?")?;

    change_descriptor_state(handle, Descriptor::DiskTitleTd, DescriptorAction::Close)?;
    change_descriptor_state(handle, Descriptor::DiskTitleTd, DescriptorAction::OpenRead)?;
    change_descriptor_state(handle, Descriptor::DiskTitleTd, DescriptorAction::Close)?;
    Ok(())
}

/// Sets a track's title. Mirrors `NetMDInterface.setTrackTitle`. `track` is 0-based.
///
/// NOTE: title sanitization is NOT applied (see UNPORTED.md).
pub fn set_track_title<T: UsbContext>(
    handle: &DeviceHandle<T>,
    track: u16,
    title: &str,
    w_char: bool,
) -> anyhow::Result<()> {
    debug!("set track title #{track}: {title:?} (wchar={w_char})");
    let wchar_value: u8 = if w_char { 3 } else { 2 };
    let descriptor = if w_char {
        Descriptor::AudioUtoc4Td
    } else {
        Descriptor::AudioUtoc1Td
    };

    let new_len = get_length_after_sjis_encode(title)?;
    // If the current title matches, skip. A rejected read means no current title.
    let old_len = match get_track_title(handle, track, w_char) {
        Ok(current) => {
            if current == title {
                return Ok(());
            }
            get_length_after_sjis_encode(&current)?
        }
        Err(_) => 0,
    };

    let sjis = encode_to_sjis(title)?;
    let sjis_hex = sjis.iter().map(|b| format!("{b:02x}")).collect::<String>();

    change_descriptor_state(handle, descriptor, DescriptorAction::OpenWrite)?;
    let query = format!(
        "00 1807 022018{:02x} {:04x} 3000 0a00 5000 {:04x} 0000 {:04x} {}",
        wchar_value, track, new_len, old_len, sjis_hex
    );
    let reply = send_query(handle, query)?;
    reply.scan("%? 1807 022018%? %?%? 3000 0a00 5000 %?%? 0000 %?%?")?;
    change_descriptor_state(handle, descriptor, DescriptorAction::Close)?;
    Ok(())
}

/// Reads disc flags. Mirrors `NetMDInterface.getDiscFlags`.
pub fn get_disc_flags<T: UsbContext>(handle: &DeviceHandle<T>) -> anyhow::Result<u8> {
    debug!("get disc flags");
    change_descriptor_state(handle, Descriptor::RootTd, DescriptorAction::OpenRead)?;
    let reply = send_query(handle, "00 1806 01101000 ff00 0001000b")?;
    let data = reply.scan("%? 1806 01101000 1000 0001000b %b")?;
    let flags = parse_u8(data[0])?;
    change_descriptor_state(handle, Descriptor::RootTd, DescriptorAction::Close)?;
    Ok(flags)
}

/// Reads disc capacity as three `[h,m,s,f]` time arrays:
/// `[recorded, total, available]`. Mirrors `NetMDInterface.getDiscCapacity`.
pub fn get_disc_capacity<T: UsbContext>(handle: &DeviceHandle<T>) -> anyhow::Result<[[u32; 4]; 3]> {
    debug!("get disc capacity");
    change_descriptor_state(handle, Descriptor::RootTd, DescriptorAction::OpenRead)?;
    let reply = send_query(handle, "00 1806 02101000 3080 0300 ff00 00000000")?;
    // 8003 relaxed to %?03 (Panasonic returns 0803). Three groups of (%W %B %B %B).
    let data = reply.scan(
        "%? 1806 02101000 3080 0300 1000 001d0000 001b %?03 0017 8000 \
         0005 %W %B %B %B 0005 %W %B %B %B 0005 %W %B %B %B",
    )?;
    // data: [w,b,b,b, w,b,b,b, w,b,b,b] (12 slices). %W is BCD word, %B is BCD byte.
    let mut result = [[0u32; 4]; 3];
    for (group, chunk) in data.chunks(4).enumerate().take(3) {
        result[group][0] = parse_bcd_u16(chunk[0])? as u32;
        result[group][1] = parse_bcd_u8(chunk[1])? as u32;
        result[group][2] = parse_bcd_u8(chunk[2])? as u32;
        result[group][3] = parse_bcd_u8(chunk[3])? as u32;
    }
    change_descriptor_state(handle, Descriptor::RootTd, DescriptorAction::Close)?;
    Ok(result)
}

/// Reads raw per-track info bytes. Mirrors `NetMDInterface._getTrackInfo`.
/// `track` is 0-based.
pub fn get_track_info<T: UsbContext>(
    handle: &DeviceHandle<T>,
    track: u16,
    p1: u16,
    p2: u16,
) -> anyhow::Result<Vec<u8>> {
    change_descriptor_state(
        handle,
        Descriptor::AudioContentsTd,
        DescriptorAction::OpenRead,
    )?;
    let query = format!(
        "00 1806 02201001 {:04x} {:04x} {:04x} ff00 00000000",
        track, p1, p2
    );
    let reply = send_query(handle, query)?;
    let data = reply.scan("%? 1806 02201001 %?%? %?%? %?%? 1000 00%?0000 %x")?;
    let raw = data[0].to_vec();
    change_descriptor_state(
        handle,
        Descriptor::AudioContentsTd,
        DescriptorAction::Close,
    )?;
    Ok(raw)
}

/// Reads a track's length as `[h, m, s, f]`. Mirrors `getTrackLength`.
pub fn get_track_length<T: UsbContext>(
    handle: &DeviceHandle<T>,
    track: u16,
) -> anyhow::Result<[u32; 4]> {
    let raw = get_track_info(handle, track, 0x3000, 0x0100)?;
    let data = scan("0001 0006 0000 %B %B %B %B", &raw)?;
    Ok([
        parse_bcd_u8(data[0])? as u32,
        parse_bcd_u8(data[1])? as u32,
        parse_bcd_u8(data[2])? as u32,
        parse_bcd_u8(data[3])? as u32,
    ])
}

/// Reads a track's `[encoding, channel_count]`. Mirrors `getTrackEncoding`.
pub fn get_track_encoding<T: UsbContext>(
    handle: &DeviceHandle<T>,
    track: u16,
) -> anyhow::Result<(u8, u8)> {
    let raw = get_track_info(handle, track, 0x3080, 0x0700)?;
    let data = scan("8007 0004 0110 %b %b", &raw)?;
    Ok((parse_u8(data[0])?, parse_u8(data[1])?))
}

/// Reads a track's flags. Mirrors `NetMDInterface.getTrackFlags`. `track` is 0-based.
pub fn get_track_flags<T: UsbContext>(handle: &DeviceHandle<T>, track: u16) -> anyhow::Result<u8> {
    change_descriptor_state(
        handle,
        Descriptor::AudioContentsTd,
        DescriptorAction::OpenRead,
    )?;
    let query = format!("00 1806 01201001 {:04x} ff00 00010008", track);
    let reply = send_query(handle, query)?;
    let data = reply.scan("%? 1806 01201001 %?%? 10 00 00010008 %b")?;
    let flags = parse_u8(data[0])?;
    change_descriptor_state(
        handle,
        Descriptor::AudioContentsTd,
        DescriptorAction::Close,
    )?;
    Ok(flags)
}

/// Erases a single track. Mirrors `NetMDInterface.eraseTrack`. `track` is 0-based.
/// DESTRUCTIVE.
pub fn erase_track<T: UsbContext>(handle: &DeviceHandle<T>, track: u16) -> anyhow::Result<()> {
    debug!("erase track #{track}");
    let query = format!("00 1840 ff01 00 201001 {:04x}", track);
    // JS does not scan the reply for this command.
    send_query(handle, query)?;
    Ok(())
}

/// Moves a track from `source` to `dest` (0-based). Mirrors `NetMDInterface.moveTrack`.
/// DESTRUCTIVE (reorders disc).
pub fn move_track<T: UsbContext>(
    handle: &DeviceHandle<T>,
    source: u16,
    dest: u16,
) -> anyhow::Result<()> {
    debug!("move track {source} -> {dest}");
    let query = format!("00 1843 ff00 00 201001 {:04x} 201001 {:04x}", source, dest);
    // JS does not scan the reply for this command.
    send_query(handle, query)?;
    Ok(())
}

/// Erases the entire disc. Mirrors `NetMDInterface.eraseDisc`. DESTRUCTIVE.
pub fn erase_disc<T: UsbContext>(handle: &DeviceHandle<T>) -> anyhow::Result<()> {
    debug!("erase disc");
    let reply = send_query(handle, "00 1840 ff 0000")?;
    reply.scan("%? 1840 00 0000")?;
    Ok(())
}

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

/// Opens then closes a descriptor TD. Mirrors `changeDescriptorState`.
pub fn change_descriptor_state<T: UsbContext>(
    handle: &DeviceHandle<T>,
    descriptor: Descriptor,
    action: DescriptorAction,
) -> anyhow::Result<()> {
    // The JS reference swallows descriptor errors; we propagate them so callers
    // can decide. Most descriptor open/close pairs are expected to succeed.
    send_query(handle, DescriptorCommand(descriptor, action))?;
    Ok(())
}

/// Reads the disc subunit identifier and returns the NetMD level byte.
///
/// Mirrors `NetMDInterface._getDiscSubunitIdentifier`. The descriptor body is
/// decoded to locate the supported-media-type specifications; the
/// implementation profile ID of media type `0x301` is the NetMD level.
pub fn get_disc_subunit_identifier<T: UsbContext>(handle: &DeviceHandle<T>) -> anyhow::Result<u8> {
    debug!("get disc subunit identifier");
    change_descriptor_state(
        handle,
        Descriptor::DiscSubUnitIdentifier,
        DescriptorAction::OpenRead,
    )?;
    let reply = send_query(handle, "00 1809 00 ff00 0000 0000")?;
    let data = reply.scan("%? 1809 00 1000 %?%? %?%? %w %b %b %b %b %w %*")?;

    // res[0]=descriptorLength(word) res[1]=generationID res[2]=sizeOfListID
    // res[3]=sizeOfObjectID res[4]=sizeOfObjectPosition res[5]=amtOfRootObjectLists(word)
    // res[6]=buffer
    let size_of_list_id = parse_u8(data[2])? as usize;
    let amt_of_root_object_lists = parse_u16(data[5])? as usize;
    let buffer = data[6];

    let mut offset = 0usize;
    let construct_multibyte = |offset: &mut usize, n: usize| -> u32 {
        let mut output: u32 = 0;
        for _ in 0..n {
            output <<= 8;
            output |= buffer[*offset] as u32;
            *offset += 1;
        }
        output
    };

    for _ in 0..amt_of_root_object_lists {
        construct_multibyte(&mut offset, size_of_list_id);
    }

    let _subunit_dependent_length = construct_multibyte(&mut offset, 2);
    let _subunit_fields_length = construct_multibyte(&mut offset, 2);
    let _attributes = buffer[offset];
    offset += 1;
    let _disc_subunit_version = buffer[offset];
    offset += 1;

    let mut net_md_level: Option<u8> = None;
    let amt_supported_media_types = buffer[offset];
    offset += 1;
    for _ in 0..amt_supported_media_types {
        let supported_media_type = construct_multibyte(&mut offset, 2);
        let implementation_profile_id = buffer[offset];
        offset += 1;
        let _media_type_attributes = buffer[offset];
        offset += 1;
        let _type_dep_length = construct_multibyte(&mut offset, 2);
        let _md_audio_version = buffer[offset];
        offset += 1;
        let _supports_md_clip = buffer[offset];
        offset += 1;

        if supported_media_type == 0x301 {
            net_md_level = Some(implementation_profile_id);
        }
    }

    change_descriptor_state(
        handle,
        Descriptor::DiscSubUnitIdentifier,
        DescriptorAction::Close,
    )?;

    net_md_level.ok_or_else(|| anyhow::anyhow!("NetMD level (media type 0x301) not found"))
}

/// Reads the raw operating status block. Mirrors `NetMDInterface.getStatus`.
pub fn get_status<T: UsbContext>(handle: &DeviceHandle<T>) -> anyhow::Result<Vec<u8>> {
    debug!("get status");
    change_descriptor_state(
        handle,
        Descriptor::OperatingStatusBlock,
        DescriptorAction::OpenRead,
    )?;
    let reply = send_query(handle, "00 1809 8001 0230 8800 0030 8804 00 ff00 00000000")?;
    let data = reply.scan("%? 1809 8001 0230 8800 0030 8804 00 1000 00090000 %x")?;
    let status = data[0].to_vec();
    change_descriptor_state(
        handle,
        Descriptor::OperatingStatusBlock,
        DescriptorAction::Close,
    )?;
    Ok(status)
}

/// Returns true when a disc is present. Mirrors `NetMDInterface.isDiscPresent`.
pub fn is_disc_present<T: UsbContext>(handle: &DeviceHandle<T>) -> anyhow::Result<bool> {
    let status = get_status(handle)?;
    Ok(status.get(4) == Some(&0x40))
}

/// Returns `[status_mode, operating_status]`. Mirrors `getFullOperatingStatus`.
///
/// WARNING (from JS reference): does not work on all devices.
pub fn get_full_operating_status<T: UsbContext>(
    handle: &DeviceHandle<T>,
) -> anyhow::Result<(u8, u16)> {
    debug!("get full operating status");
    change_descriptor_state(
        handle,
        Descriptor::OperatingStatusBlock,
        DescriptorAction::OpenRead,
    )?;
    let reply = send_query(
        handle,
        "00 1809 8001 0330 8802 0030 8805 0030 8806 00 ff00 00000000",
    )?;
    let data = reply.scan(
        "%? 1809 8001 0330 8802 0030 8805 0030 8806 00 1000 00%?0000 00%b 8806 %x",
    )?;
    let status_mode = parse_u8(data[0])?;
    let operating_status = data[1];
    change_descriptor_state(
        handle,
        Descriptor::OperatingStatusBlock,
        DescriptorAction::Close,
    )?;
    if operating_status.len() < 2 {
        anyhow::bail!("unparsable operating status");
    }
    let operating_status_number = ((operating_status[0] as u16) << 8) | operating_status[1] as u16;
    Ok((status_mode, operating_status_number))
}

/// Returns the operating status number. Mirrors `getOperatingStatus`.
pub fn get_operating_status<T: UsbContext>(handle: &DeviceHandle<T>) -> anyhow::Result<u16> {
    Ok(get_full_operating_status(handle)?.1)
}

pub fn read_reply<T: UsbContext>(handle: &DeviceHandle<T>) -> Result<ReadRequestData, rusb::Error> {
    // header is 4 bytes, of which the third byte is the length of the next message.
    // Poll until a non-zero length is reported, doubling the wait each attempt.
    let mut reply_header = read_reply_length(handle)?;
    let mut i: u32 = 0;
    while reply_header.len() == 0 {
        sleep(Duration::from_millis(READ_REPLY_POLL_INTERVAL_MS << i));
        reply_header = read_reply_length(handle)?;
        i += 1;
    }

    let mut reply = ReadRequestData::new(reply_header.len());

    handle.read_control(
        request_type(
            rusb::Direction::In,
            rusb::RequestType::Vendor,
            rusb::Recipient::Interface,
        ),
        0x81,
        0,
        0,
        &mut reply.0,
        Duration::from_millis(USB_TIMEOUT_MILLIS),
    )?;

    trace!("  RX ← {:02x?}", reply.0);

    Ok(reply)
}
