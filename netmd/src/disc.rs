use log::debug;
use rusb::{DeviceHandle, UsbContext};

use crate::{
    descriptor::{change_descriptor_state, Descriptor, DescriptorAction},
    device::SHARP_VENDOR_ID,
    scan::scan,
    title::{sanitize_full_width_title, sanitize_half_width_title},
    transport::send_query,
    types::DiscFlags,
    util::{
        encode_to_sjis, get_length_after_sjis_encode, parse_bcd_u16, parse_bcd_u8, parse_string,
        parse_u16, parse_u8,
    },
};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::SONY_VENDOR_ID;
    use crate::types::DiscFlag;

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
