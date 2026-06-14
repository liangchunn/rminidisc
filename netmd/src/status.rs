use log::{debug, trace};
use rusb::{DeviceHandle, UsbContext};

use crate::{
    descriptor::{change_descriptor_state, Descriptor, DescriptorAction},
    error::{NetMDError, Result},
    transport::send_query,
    types::{FullOperatingStatus, OperatingStatus},
    util::parse_u8,
};

/// Reads the raw operating status block. Mirrors `NetMDInterface.getStatus`.
pub fn get_status<T: UsbContext>(handle: &DeviceHandle<T>) -> Result<Vec<u8>> {
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
pub fn is_disc_present<T: UsbContext>(handle: &DeviceHandle<T>) -> Result<bool> {
    trace!("check disc present");
    let status = get_status(handle)?;
    Ok(status.get(4) == Some(&0x40))
}

/// Returns the full operating status. Mirrors `getFullOperatingStatus`.
///
/// WARNING (from JS reference): does not work on all devices.
pub fn get_full_operating_status<T: UsbContext>(
    handle: &DeviceHandle<T>,
) -> Result<FullOperatingStatus> {
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
        return Err(NetMDError::UnexpectedResponse(
            "unparsable operating status".to_string(),
        ));
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
pub fn get_operating_status<T: UsbContext>(handle: &DeviceHandle<T>) -> Result<OperatingStatus> {
    Ok(get_full_operating_status(handle)?.status)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
