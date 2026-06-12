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

use anyhow::{bail, Context};
use log::{debug, trace};
use rusb::{request_type, Device, DeviceHandle, GlobalContext, UsbContext};

use crate::{
    descriptor::{Descriptor, DescriptorAction, DescriptorCommand},
    error::NetMDError,
    query::Query,
    scan::scan,
    title::{sanitize_full_width_title, sanitize_half_width_title},
    types::{
        ProtocolReply, ReadRequestData, ReadRequestHeader, INTERIM_RETRY_INTERVAL_MS,
        MAX_INTERIM_ATTEMPTS, READ_REPLY_POLL_INTERVAL_MS, USB_TIMEOUT_MILLIS,
    },
    util::{
        encode_to_sjis, get_length_after_sjis_encode, parse_bcd_u16, parse_bcd_u8, parse_string,
        parse_u16, parse_u8,
    },
};

pub mod crypto;
pub mod descriptor;
pub mod ekb;
pub mod error;
pub mod query;
pub mod scan;
pub mod title;
pub mod track;
pub mod types;
pub mod util;
pub mod wav;

// --- Public re-exports for ergonomic access from downstream crates. ---
// (The internal `use crate::descriptor::{...}` above already brings the
// descriptor types into this module's namespace; the modules are also public,
// so downstream crates can reach everything via `netmd::descriptor::...` etc.)
pub use error::NetMDError as Error;
pub use types::{
    ChannelCount, Channels, DiscFlag, DiscFlags, DiscFormat, Encoding, FullOperatingStatus,
    NetMDLevel, OperatingStatus, ProtocolReply as Status, TrackFlag, Wireformat, FRAME_SIZE,
};
pub use util::{format_time_from_frames, time_to_frames};

/// Sony USB vendor ID.
pub const SONY_VENDOR_ID: u16 = 0x054c;
/// Sharp USB vendor ID. Sharp devices need a different disc-title descriptor flow.
pub const SHARP_VENDOR_ID: u16 = 0x04dd;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeviceFlags {
    pub native_mono_upload: bool,
    pub native_lp_encoding: bool,
}

impl DeviceFlags {
    const fn empty() -> Self {
        Self {
            native_mono_upload: false,
            native_lp_encoding: false,
        }
    }

    const fn native_mono_upload() -> Self {
        Self {
            native_mono_upload: true,
            native_lp_encoding: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeviceDefinition {
    pub vendor_id: u16,
    pub product_id: u16,
    pub name: &'static str,
    pub flags: DeviceFlags,
}

pub const SUPPORTED_DEVICES: &[DeviceDefinition] = &[
    DeviceDefinition {
        vendor_id: 0x04dd,
        product_id: 0x7202,
        name: "Sharp IM-MT899H",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x04dd,
        product_id: 0x9013,
        name: "Sharp IM-DR400",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x04dd,
        product_id: 0x9014,
        name: "Sharp IM-DR80",
        flags: DeviceFlags::native_mono_upload(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0034,
        name: "Sony PCLK-XX",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0036,
        name: "Sony",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0075,
        name: "Sony MZ-N1",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x007c,
        name: "Sony",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0080,
        name: "Sony LAM-1",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0081,
        name: "Sony MDS-JB980/MDS-NT1/MDS-JE780",
        flags: DeviceFlags::native_mono_upload(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0084,
        name: "Sony MZ-N505",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0085,
        name: "Sony MZ-S1",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0086,
        name: "Sony MZ-N707",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x008e,
        name: "Sony CMT-C7NT",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0097,
        name: "Sony PCGA-MDN1",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x00ad,
        name: "Sony CMT-L7HD",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x00c6,
        name: "Sony MZ-N10",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x00c7,
        name: "Sony MZ-N910",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x00c8,
        name: "Sony MZ-N710/NF810",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x00c9,
        name: "Sony MZ-N510/N610",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x00ca,
        name: "Sony MZ-NE410/NF520D",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x00e7,
        name: "Sony CMT-M333NT/M373NT",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x00eb,
        name: "Sony MZ-NE810/NE910",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0101,
        name: "Sony LAM",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0113,
        name: "Aiwa AM-NX1",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x013f,
        name: "Sony MDS-S500",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x014c,
        name: "Aiwa AM-NX9",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x017e,
        name: "Sony MZ-NH1",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0180,
        name: "Sony MZ-NH3D",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0182,
        name: "Sony MZ-NH900",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0184,
        name: "Sony MZ-NH700/NH800",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0186,
        name: "Sony MZ-NH600",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0187,
        name: "Sony MZ-NH600D",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0188,
        name: "Sony MZ-N920",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x018a,
        name: "Sony LAM-3",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x01e9,
        name: "Sony MZ-DH10P",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0219,
        name: "Sony MZ-RH10",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x021b,
        name: "Sony MZ-RH710/MZ-RH910",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x021d,
        name: "Sony CMT-AH10",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x022c,
        name: "Sony CMT-AH10",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x023c,
        name: "Sony DS-HMD1",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0286,
        name: "Sony MZ-RH1",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x011a,
        name: "Sony CMT-SE7",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x054c,
        product_id: 0x0148,
        name: "Sony MDS-A1",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x0b28,
        product_id: 0x1004,
        name: "Kenwood MDX-J9",
        flags: DeviceFlags::empty(),
    },
    DeviceDefinition {
        vendor_id: 0x04da,
        product_id: 0x23b3,
        name: "Panasonic SJ-MR250",
        flags: DeviceFlags::native_mono_upload(),
    },
    DeviceDefinition {
        vendor_id: 0x04da,
        product_id: 0x23b6,
        name: "Panasonic SJ-MR270",
        flags: DeviceFlags::native_mono_upload(),
    },
    DeviceDefinition {
        vendor_id: 0x0411,
        product_id: 0x0083,
        name: "Buffalo MD-HUSB",
        flags: DeviceFlags::empty(),
    },
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeviceSelector {
    pub vendor_id: u16,
    pub product_id: u16,
}

impl DeviceSelector {
    pub const fn new(vendor_id: u16, product_id: u16) -> Self {
        Self {
            vendor_id,
            product_id,
        }
    }
}

pub fn supported_device(vendor_id: u16, product_id: u16) -> Option<&'static DeviceDefinition> {
    SUPPORTED_DEVICES
        .iter()
        .find(|device| device.vendor_id == vendor_id && device.product_id == product_id)
}

/// Opens one connected supported NetMD device and claims its interface.
///
/// If exactly one supported device is connected, it is selected automatically.
/// If multiple supported devices are connected, callers must pass a selector.
pub fn open_device() -> anyhow::Result<DeviceHandle<GlobalContext>> {
    open_device_matching(None)
}

pub fn open_device_matching(
    selector: Option<DeviceSelector>,
) -> anyhow::Result<DeviceHandle<GlobalContext>> {
    let devices = rusb::devices()?
        .iter()
        .filter_map(connected_supported_device)
        .collect::<Vec<_>>();

    let devices = devices
        .into_iter()
        .filter(|connected| {
            selector
                .map(|selector| {
                    connected.definition.vendor_id == selector.vendor_id
                        && connected.definition.product_id == selector.product_id
                })
                .unwrap_or(true)
        })
        .collect::<Vec<_>>();

    let device = match devices.as_slice() {
        [] => {
            if let Some(selector) = selector {
                bail!(
                    "cannot find supported device {:04x}:{:04x}",
                    selector.vendor_id,
                    selector.product_id
                );
            }
            bail!("cannot find supported NetMD device");
        }
        [device] => device,
        _ => bail!(
            "multiple supported NetMD devices connected; specify one with --device <vid:pid>:\n{}",
            devices
                .iter()
                .map(ConnectedDevice::display)
                .collect::<Vec<_>>()
                .join("\n")
        ),
    };

    let device_desc = device.device.device_descriptor()?;
    let device_name = device.definition.name;
    let device_id = format!(
        "{:04x}:{:04x} {device_name}",
        device_desc.vendor_id(),
        device_desc.product_id()
    );
    let handle = device
        .device
        .open()
        .with_context(|| format!("failed to open USB device {device_id}"))?;

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
            "opened {:04x}:{:04x} {} ({})",
            device_desc.vendor_id(),
            device_desc.product_id(),
            device_name,
            manufacturer.join(", ")
        );
    }

    handle
        .claim_interface(0)
        .with_context(|| format!("failed to claim USB interface 0 on {device_id}"))?;
    Ok(handle)
}

struct ConnectedDevice {
    device: Device<GlobalContext>,
    definition: &'static DeviceDefinition,
}

impl ConnectedDevice {
    fn display(&self) -> String {
        format!(
            "  {:04x}:{:04x} {}",
            self.definition.vendor_id, self.definition.product_id, self.definition.name
        )
    }
}

fn connected_supported_device(device: Device<GlobalContext>) -> Option<ConnectedDevice> {
    let desc = device.device_descriptor().ok()?;
    supported_device(desc.vendor_id(), desc.product_id())
        .map(|definition| ConnectedDevice { device, definition })
}

/// Releases the claimed interface. Mirrors the runner's previous teardown.
pub fn close_device<T: UsbContext>(handle: &DeviceHandle<T>) -> anyhow::Result<()> {
    handle.release_interface(0)?;
    Ok(())
}

/// Returns the `(vendor_id, product_id)` of the device behind a handle. Used for
/// EKB selection during secure download.
pub fn device_ids<T: UsbContext>(handle: &DeviceHandle<T>) -> anyhow::Result<(u16, u16)> {
    let desc = handle.device().device_descriptor()?;
    Ok((desc.vendor_id(), desc.product_id()))
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
pub fn get_disk_title<T: UsbContext>(
    handle: &DeviceHandle<T>,
    w_char: bool,
) -> anyhow::Result<String> {
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
pub fn get_disc_title<T: UsbContext>(
    handle: &DeviceHandle<T>,
    w_char: bool,
) -> anyhow::Result<String> {
    Ok(disc_title_from_raw(
        &get_disk_title(handle, w_char)?,
        w_char,
    ))
}

fn disc_title_from_raw(raw_title: &str, w_char: bool) -> String {
    let delim = if w_char { "／／" } else { "//" };
    let title_marker = if w_char { "０；" } else { "0;" };

    if raw_title.ends_with(delim) {
        let first_entry = raw_title.split(delim).next().unwrap_or("");
        if let Some(stripped) = first_entry.strip_prefix(title_marker) {
            stripped.to_string()
        } else {
            String::new()
        }
    } else {
        raw_title.to_string()
    }
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
    change_descriptor_state(handle, Descriptor::AudioContentsTd, DescriptorAction::Close)?;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiscTitleWriteFlow {
    Standard,
    Sharp,
}

fn disc_title_write_flow(vendor_id: u16) -> DiscTitleWriteFlow {
    if vendor_id == SHARP_VENDOR_ID {
        DiscTitleWriteFlow::Sharp
    } else {
        DiscTitleWriteFlow::Standard
    }
}

fn sanitize_title(title: &str, w_char: bool) -> String {
    if w_char {
        sanitize_full_width_title(title)
    } else {
        sanitize_half_width_title(title)
    }
}

/// Sets the disc title. Mirrors `NetMDInterface.setDiscTitle`.
pub fn set_disc_title<T: UsbContext>(
    handle: &DeviceHandle<T>,
    title: &str,
    w_char: bool,
) -> anyhow::Result<()> {
    debug!("set disc title: {title:?} (wchar={w_char})");
    let title = sanitize_title(title, w_char);
    let current_title = get_disk_title(handle, w_char)?;
    if current_title == title.as_str() {
        // Setting the same title causes problems with the LAM.
        return Ok(());
    }
    let old_len = get_length_after_sjis_encode(&current_title)?;
    let new_len = get_length_after_sjis_encode(&title)?;
    let wchar_value: u8 = if w_char { 1 } else { 0 };
    let sjis = encode_to_sjis(&title)?;
    let sjis_hex = sjis.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let flow = disc_title_write_flow(handle.device().device_descriptor()?.vendor_id());

    match flow {
        DiscTitleWriteFlow::Sharp => {
            change_descriptor_state(
                handle,
                Descriptor::AudioUtoc1Td,
                DescriptorAction::OpenWrite,
            )?;
        }
        DiscTitleWriteFlow::Standard => {
            change_descriptor_state(handle, Descriptor::DiskTitleTd, DescriptorAction::Close)?;
            change_descriptor_state(handle, Descriptor::DiskTitleTd, DescriptorAction::OpenWrite)?;
        }
    }

    let query = format!(
        "00 1807 02201801 00{:02x} 3000 0a00 5000 {:04x} 0000 {:04x} {}",
        wchar_value, new_len, old_len, sjis_hex
    );
    let reply = send_query(handle, query)?;
    reply.scan("%? 1807 02201801 00%? 3000 0a00 5000 %?%? 0000 %?%?")?;

    match flow {
        DiscTitleWriteFlow::Sharp => {
            change_descriptor_state(handle, Descriptor::AudioUtoc1Td, DescriptorAction::Close)?;
        }
        DiscTitleWriteFlow::Standard => {
            change_descriptor_state(handle, Descriptor::DiskTitleTd, DescriptorAction::Close)?;
            change_descriptor_state(handle, Descriptor::DiskTitleTd, DescriptorAction::OpenRead)?;
            change_descriptor_state(handle, Descriptor::DiskTitleTd, DescriptorAction::Close)?;
        }
    }
    Ok(())
}

/// Renames the user-facing disc title while preserving any group metadata in the
/// raw disc-title field.
pub fn rename_disc<T: UsbContext>(
    handle: &DeviceHandle<T>,
    title: &str,
    w_char: bool,
) -> anyhow::Result<()> {
    let title = sanitize_title(title, w_char);
    let raw_title = get_disk_title(handle, w_char)?;
    let current_title = disc_title_from_raw(&raw_title, w_char);
    if title == current_title {
        return Ok(());
    }

    set_disc_title(
        handle,
        &renamed_disc_raw_title(&raw_title, &title, w_char),
        w_char,
    )
}

fn renamed_disc_raw_title(raw_title: &str, title: &str, w_char: bool) -> String {
    let delim = if w_char { "／／" } else { "//" };
    let title_marker = if w_char { "０；" } else { "0;" };

    if raw_title.contains(delim) {
        if raw_title.starts_with(title_marker) {
            let (_, rest) = raw_title.split_once(delim).unwrap_or(("", ""));
            if title.is_empty() {
                rest.to_string()
            } else {
                format!("{title_marker}{title}{delim}{rest}")
            }
        } else {
            format!("{title_marker}{title}{delim}{raw_title}")
        }
    } else {
        title.to_string()
    }
}

/// Sets a track's title. Mirrors `NetMDInterface.setTrackTitle`. `track` is 0-based.
pub fn set_track_title<T: UsbContext>(
    handle: &DeviceHandle<T>,
    track: u16,
    title: &str,
    w_char: bool,
) -> anyhow::Result<()> {
    debug!("set track title #{track}: {title:?} (wchar={w_char})");
    let title = sanitize_title(title, w_char);
    let wchar_value: u8 = if w_char { 3 } else { 2 };
    let descriptor = if w_char {
        Descriptor::AudioUtoc4Td
    } else {
        Descriptor::AudioUtoc1Td
    };

    let new_len = get_length_after_sjis_encode(&title)?;
    // If the current title matches, skip. A rejected read means no current title.
    let old_len = match get_track_title(handle, track, w_char) {
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
pub fn get_disc_flags<T: UsbContext>(handle: &DeviceHandle<T>) -> anyhow::Result<DiscFlags> {
    debug!("get disc flags");
    change_descriptor_state(handle, Descriptor::RootTd, DescriptorAction::OpenRead)?;
    let reply = send_query(handle, "00 1806 01101000 ff00 0001000b")?;
    let data = reply.scan("%? 1806 01101000 1000 0001000b %b")?;
    let flags = DiscFlags::from_bits(parse_u8(data[0])?);
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
    change_descriptor_state(handle, Descriptor::AudioContentsTd, DescriptorAction::Close)?;
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

/// Reads a track's `(encoding, channel_count)`. Mirrors `getTrackEncoding`.
pub fn get_track_encoding<T: UsbContext>(
    handle: &DeviceHandle<T>,
    track: u16,
) -> anyhow::Result<(Encoding, ChannelCount)> {
    let raw = get_track_info(handle, track, 0x3080, 0x0700)?;
    let data = scan("8007 0004 0110 %b %b", &raw)?;
    Ok((
        track_encoding_from_byte(parse_u8(data[0])?)?,
        channel_count_from_byte(parse_u8(data[1])?)?,
    ))
}

fn track_encoding_from_byte(value: u8) -> anyhow::Result<Encoding> {
    match value {
        value if value == Encoding::Sp as u8 => Ok(Encoding::Sp),
        value if value == Encoding::Lp2 as u8 => Ok(Encoding::Lp2),
        value if value == Encoding::Lp4 as u8 => Ok(Encoding::Lp4),
        _ => bail!("unknown track encoding: 0x{value:02x}"),
    }
}

fn channel_count_from_byte(value: u8) -> anyhow::Result<ChannelCount> {
    match value {
        0x00 => Ok(ChannelCount::Stereo),
        0x01 => Ok(ChannelCount::Mono),
        _ => bail!("unknown channel count: 0x{value:02x}"),
    }
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
    change_descriptor_state(handle, Descriptor::AudioContentsTd, DescriptorAction::Close)?;
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

/// Returns the full operating status. Mirrors `getFullOperatingStatus`.
///
/// WARNING (from JS reference): does not work on all devices.
pub fn get_full_operating_status<T: UsbContext>(
    handle: &DeviceHandle<T>,
) -> anyhow::Result<FullOperatingStatus> {
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
    let data =
        reply.scan("%? 1809 8001 0330 8802 0030 8805 0030 8806 00 1000 00%?0000 00%b 8806 %x")?;
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
    Ok(FullOperatingStatus {
        mode: status_mode,
        status: operating_status_from_u16(operating_status_number),
    })
}

fn operating_status_from_u16(value: u16) -> OperatingStatus {
    match value {
        0xc5ff => OperatingStatus::Ready,
        0xffff => OperatingStatus::BlankDisc,
        _ => OperatingStatus::Unknown(value),
    }
}

/// Returns the operating status. Mirrors `getOperatingStatus`.
pub fn get_operating_status<T: UsbContext>(
    handle: &DeviceHandle<T>,
) -> anyhow::Result<OperatingStatus> {
    Ok(get_full_operating_status(handle)?.status)
}

pub fn read_reply<T: UsbContext>(handle: &DeviceHandle<T>) -> Result<ReadRequestData, rusb::Error> {
    // header is 4 bytes, of which the third byte is the length of the next message.
    // Poll until a non-zero length is reported, doubling the wait each attempt.
    let mut reply_header = read_reply_length(handle)?;
    let mut i: u32 = 0;
    while reply_header.is_empty() {
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

    // Mirror `_readReply` (`netmd.ts:206`): refresh the reply-length register
    // after reading the payload. Harmless for short commands and required for
    // the device's flow control during the secure-download reply sequence.
    let _ = read_reply_length(handle);

    Ok(reply)
}

/// Reads a reply after a long-running bulk transfer (track commit).
///
/// Unlike [`read_reply`], the reply-length poll here tolerates USB timeouts:
/// the device can take several seconds to finalize the track before it produces
/// a reply, during which the control-IN length read may time out. Each timeout
/// is treated as "not ready yet" and retried up to an overall budget.
fn read_reply_after_bulk<T: UsbContext>(
    handle: &DeviceHandle<T>,
) -> Result<ReadRequestData, NetMDError> {
    const MAX_POLLS: u32 = 200; // ~ up to a minute with the sleeps below.
    let mut polls = 0u32;
    let header = loop {
        match read_reply_length(handle) {
            Ok(h) if !h.is_empty() => break h,
            Ok(_) => {
                // Not ready yet.
            }
            Err(rusb::Error::Timeout) => {
                // Device still busy; keep polling.
            }
            Err(e) => return Err(NetMDError::Usb(e)),
        }
        polls += 1;
        if polls >= MAX_POLLS {
            return Err(NetMDError::Usb(rusb::Error::Timeout));
        }
        sleep(Duration::from_millis(200));
    };

    let mut reply = ReadRequestData::new(header.len());
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

// ===========================================================================
// Secure download (track-write) pipeline. Ported from `netmd-interface.ts`
// secure-session methods + `MDSession`/`MDTrack`. Track-read (saveTrackToArray)
// is intentionally NOT ported (RH1/M200-only hardware). See UNPORTED.md §3.
// ===========================================================================

use crate::crypto::EncryptedPacket;
use crate::query::QueryBuilder;

/// USB bulk OUT endpoint address for track data. WebUSB endpoint `0x02`
/// (`netmd.ts:6`) maps to rusb endpoint address `0x02`.
pub const BULK_WRITE_ENDPOINT: u8 = 0x02;

/// Common command prefix for all secure-session commands.
///
/// The leading `00` is the status/command byte that `NetMDInterface.sendCommand`
/// (`netmd.ts:226`) prepends to every query before the control write. The
/// existing non-secure commands in this crate spell that byte out explicitly in
/// their hex strings (e.g. `"00 1806 ..."`); the secure commands include it here
/// so the device echoes back `1800 080046 ...` with the status replacing the
/// leading `00`, matching the reply scan templates.
const SECURE_PREFIX: &str = "00 1800 080046 f0030103";

/// Maximum bytes per single bulk-OUT libusb call.
///
/// NOTE: this is a deliberate deviation from the JS reference. `NetMD.writeBulk`
/// (`netmd.ts:231`) hands the entire packet to a single WebUSB `transferOut`
/// call and lets the browser split it into endpoint-sized USB transactions.
/// libusb/rusb does not do that splitting for us: a single multi-MB `write_bulk`
/// (the first SP packet is ~1 MB, and the whole payload can be ~79 MB) stalls on
/// some hosts (observed on macOS). We therefore split into `0x10000` pieces,
/// matching the chunk size `readBulk` uses for reads (`netmd-interface.ts:714`).
const BULK_WRITE_CHUNK: usize = 0x10000;

/// Writes data to the bulk OUT endpoint. Mirrors `NetMD.writeBulk` (`netmd.ts:231`),
/// except it splits the write into [`BULK_WRITE_CHUNK`]-sized libusb calls (see
/// the constant's docs) so large SP payloads transfer reliably.
pub fn write_bulk<T: UsbContext>(handle: &DeviceHandle<T>, data: &[u8]) -> anyhow::Result<()> {
    let mut written = 0;
    while written < data.len() {
        let end = (written + BULK_WRITE_CHUNK).min(data.len());
        let chunk = &data[written..end];
        let mut off = 0;
        while off < chunk.len() {
            let n = handle.write_bulk(
                BULK_WRITE_ENDPOINT,
                &chunk[off..],
                Duration::from_millis(USB_TIMEOUT_MILLIS * 20),
            )?;
            if n == 0 {
                anyhow::bail!("bulk write made no progress");
            }
            off += n;
        }
        written += chunk.len();
    }
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

    // formatQuery('… 12 ff %w 0000 %w %d %d %d 00000000 %* %*',
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
            // big-endian u32 (`sendTrack` reverses the LE buffer on LE hosts —
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
    for _ in 0..50 {
        match get_operating_status(handle) {
            Ok(OperatingStatus::Ready) | Ok(OperatingStatus::BlankDisc) => break,
            _ => sleep(Duration::from_millis(200)),
        }
    }
    // Best-effort: forget any prior session.
    let _ = session_key_forget(handle);
    let _ = leave_secure_session(handle);

    acquire(handle)?;
    // On Sharp devices this doesn't work; ignore errors.
    let _ = disable_new_track_protection(handle, 1);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disc_title_write_flow_uses_sharp_branch_only_for_sharp_vendor() {
        assert_eq!(
            disc_title_write_flow(SHARP_VENDOR_ID),
            DiscTitleWriteFlow::Sharp
        );
        assert_eq!(
            disc_title_write_flow(SONY_VENDOR_ID),
            DiscTitleWriteFlow::Standard
        );
        assert_eq!(disc_title_write_flow(0x1234), DiscTitleWriteFlow::Standard);
    }

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

    #[test]
    fn disc_flag_decodes_wire_values() {
        let writable = DiscFlags::from_bits(0x10);
        assert!(writable.contains(DiscFlag::Writable));
        assert!(writable.is_writable());
        assert!(!writable.is_write_protected());
        assert_eq!(writable.unknown_bits(), 0x00);

        let protected = DiscFlags::from_bits(0x50);
        assert!(protected.contains(DiscFlag::Writable));
        assert!(!protected.is_writable());
        assert!(protected.is_write_protected());
        assert_eq!(protected.unknown_bits(), 0x00);

        let unknown = DiscFlags::from_bits(0x51);
        assert_eq!(unknown.raw(), 0x51);
        assert_eq!(unknown.unknown_bits(), 0x01);
    }

    #[test]
    fn operating_status_decodes_wire_values() {
        assert_eq!(operating_status_from_u16(0xc5ff), OperatingStatus::Ready);
        assert_eq!(
            operating_status_from_u16(0xffff),
            OperatingStatus::BlankDisc
        );
        assert_eq!(
            operating_status_from_u16(0x1234),
            OperatingStatus::Unknown(0x1234)
        );
        assert_eq!(OperatingStatus::Ready.raw(), 0xc5ff);
        assert_eq!(OperatingStatus::BlankDisc.raw(), 0xffff);
        assert_eq!(OperatingStatus::Unknown(0x1234).raw(), 0x1234);
    }

    #[test]
    fn disc_title_from_raw_extracts_group_title() {
        assert_eq!(disc_title_from_raw("0;Disc//1-2;Group//", false), "Disc");
        assert_eq!(disc_title_from_raw("1-2;Group//", false), "");
        assert_eq!(disc_title_from_raw("Disc", false), "Disc");
    }

    #[test]
    fn renamed_disc_raw_title_preserves_group_metadata() {
        assert_eq!(
            renamed_disc_raw_title("0;Old//1-2;Group//", "New", false),
            "0;New//1-2;Group//"
        );
        assert_eq!(
            renamed_disc_raw_title("1-2;Group//", "New", false),
            "0;New//1-2;Group//"
        );
        assert_eq!(
            renamed_disc_raw_title("0;Old//1-2;Group//", "", false),
            "1-2;Group//"
        );
        assert_eq!(renamed_disc_raw_title("Old", "New", false), "New");
    }

    #[test]
    fn renamed_disc_raw_title_supports_full_width_groups() {
        assert_eq!(
            renamed_disc_raw_title("０；Ｏｌｄ／／１－２；Ｇｒｏｕｐ／／", "Ｎｅｗ", true),
            "０；Ｎｅｗ／／１－２；Ｇｒｏｕｐ／／"
        );
    }
}
