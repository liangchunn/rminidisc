use std::fmt::Display;

use crate::scan::scan;

pub const USB_TIMEOUT_MILLIS: u64 = 500;

#[repr(u8)]
#[derive(Debug)]
pub enum ProtocolReply {
    // NetMD Protocol return status (first byte of request)
    Control = 0x00,
    Status = 0x01,
    SpecificInquiry = 0x02,
    Notify = 0x03,
    GeneralInquiry = 0x04,
    //  ... (first byte of response)
    NotImplemented = 0x08,
    Accepted = 0x09,
    Rejected = 0x0a,
    InTransition = 0x0b,
    Implemented = 0x0c,
    Changed = 0x0d,
    Interim = 0x0f,
}

impl From<u8> for ProtocolReply {
    fn from(value: u8) -> Self {
        match value {
            0x00 => ProtocolReply::Control,
            0x01 => ProtocolReply::Status,
            0x02 => ProtocolReply::SpecificInquiry,
            0x03 => ProtocolReply::Notify,
            0x04 => ProtocolReply::GeneralInquiry,
            0x08 => ProtocolReply::NotImplemented,
            0x09 => ProtocolReply::Accepted,
            0x0a => ProtocolReply::Rejected,
            0x0b => ProtocolReply::InTransition,
            0x0c => ProtocolReply::Implemented,
            0x0d => ProtocolReply::Changed,
            0x0f => ProtocolReply::Interim,
            _ => unimplemented!(),
        }
    }
}

pub struct ReadRequestHeader(pub [u8; 4]);

impl ReadRequestHeader {
    pub fn new() -> Self {
        ReadRequestHeader([0; 4])
    }

    pub fn len(&self) -> usize {
        self.0[2] as usize
    }

    // pub fn status(&self) -> ProtocolReply {
    //     self.0[0].into()
    // }
}

#[derive(Debug)]
pub struct ReadRequestData(pub Vec<u8>);

impl ReadRequestData {
    pub fn scan<'b, 'a: 'b>(&'a self, template: &'a str) -> Result<Vec<&'b [u8]>, anyhow::Error> {
        scan(template, &self.0)
    }
}

impl ReadRequestData {
    pub fn new(size: usize) -> Self {
        ReadRequestData(vec![0; size])
    }
}

impl Display for ReadRequestData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[")?;
        for byte in &self.0 {
            write!(f, "0x{:02x}, ", byte)?;
        }
        write!(f, "]")?;
        Ok(())
    }
}
