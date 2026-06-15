//! Operating-status polling.
//!
//! Low-level status reads ([`NetMD::get_status`], [`NetMD::is_disc_present`],
//! [`NetMD::get_operating_status`], [`NetMD::get_full_operating_status`]) used
//! by higher-level snapshots such as [`NetMD::get_device_status`].

use log::{debug, trace};

use crate::{
    descriptor::{Descriptor, DescriptorAction},
    error::Result,
    query::QueryBuilder,
    types::{FullOperatingStatus, OperatingStatus},
    util::parse_u8,
};

use super::NetMD;

impl NetMD {
    /// Reads the raw operating status block. Mirrors `NetMDInterface.getStatus`.
    pub fn get_status(&self) -> Result<Vec<u8>> {
        debug!("get status");
        self.change_descriptor_state(Descriptor::OperatingStatusBlock, DescriptorAction::OpenRead)?;
        let reply = self.send_query(
            QueryBuilder::new().raw("00 1809 8001 0230 8800 0030 8804 00 ff00 00000000")?,
        )?;
        let data = reply.scan("%? 1809 8001 0230 8800 0030 8804 00 1000 00090000 %x")?;
        let status = data[0].to_vec();
        self.change_descriptor_state(Descriptor::OperatingStatusBlock, DescriptorAction::Close)?;
        Ok(status)
    }

    /// Returns true when a disc is present. Mirrors `NetMDInterface.isDiscPresent`.
    pub fn is_disc_present(&self) -> Result<bool> {
        trace!("check disc present");
        let status = self.get_status()?;
        Ok(status.get(4) == Some(&0x40))
    }

    /// Returns the full operating status. Mirrors `getFullOperatingStatus`.
    ///
    /// WARNING (from JS reference): does not work on all devices.
    pub fn get_full_operating_status(&self) -> Result<FullOperatingStatus> {
        debug!("get full operating status");
        self.change_descriptor_state(Descriptor::OperatingStatusBlock, DescriptorAction::OpenRead)?;
        let reply = self.send_query(
            QueryBuilder::new()
                .raw("00 1809 8001 0330 8802 0030 8805 0030 8806 00 ff00 00000000")?,
        )?;
        let data = reply
            .scan("%? 1809 8001 0330 8802 0030 8805 0030 8806 00 1000 00%?0000 00%b 8806 %x")?;
        let status_mode = parse_u8(data[0])?;
        let operating_status = data[1];
        self.change_descriptor_state(Descriptor::OperatingStatusBlock, DescriptorAction::Close)?;
        if operating_status.len() < 2 {
            return Err(crate::error::NetMDError::UnexpectedResponse(
                "unparsable operating status".to_string(),
            ));
        }
        let operating_status_number =
            ((operating_status[0] as u16) << 8) | operating_status[1] as u16;
        Ok(FullOperatingStatus {
            mode: status_mode,
            status: operating_status_from_u16(operating_status_number),
        })
    }

    /// Returns the operating status. Mirrors `getOperatingStatus`.
    pub fn get_operating_status(&self) -> Result<OperatingStatus> {
        Ok(self.get_full_operating_status()?.status)
    }
}

fn operating_status_from_u16(value: u16) -> OperatingStatus {
    match value {
        0xc5ff => OperatingStatus::Ready,
        0xffff => OperatingStatus::BlankDisc,
        _ => OperatingStatus::Unknown(value),
    }
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
