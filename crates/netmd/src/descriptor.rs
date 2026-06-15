use log::trace;

use crate::error::{NetMDError, Result};
use crate::query::Query;

use super::NetMD;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Descriptor {
    DiskTitleTd,
    AudioUtoc1Td,
    AudioUtoc4Td,
    DsiTd,
    AudioContentsTd,
    RootTd,
    DiscSubUnitIdentifier,
    OperatingStatusBlock,
}

impl Descriptor {
    fn as_str(&self) -> &str {
        match self {
            Descriptor::DiskTitleTd => "10 1801",
            Descriptor::AudioUtoc1Td => "10 1802",
            Descriptor::AudioUtoc4Td => "10 1803",
            Descriptor::DsiTd => "10 1804",
            Descriptor::AudioContentsTd => "10 1001",
            Descriptor::RootTd => "10 1000",
            Descriptor::DiscSubUnitIdentifier => "00",
            Descriptor::OperatingStatusBlock => "80 00",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescriptorAction {
    OpenRead,
    OpenWrite,
    Close,
}

impl DescriptorAction {
    fn as_str(&self) -> &str {
        match self {
            DescriptorAction::OpenRead => "01",
            DescriptorAction::OpenWrite => "03",
            DescriptorAction::Close => "00",
        }
    }
}

pub struct DescriptorCommand(pub Descriptor, pub DescriptorAction);

impl NetMD {
    /// Opens then closes a descriptor TD. Mirrors `changeDescriptorState`.
    pub fn change_descriptor_state(
        &self,
        descriptor: Descriptor,
        action: DescriptorAction,
    ) -> Result<()> {
        trace!("change descriptor state: {descriptor:?} {action:?}");
        self.send_query(DescriptorCommand(descriptor, action))?;
        Ok(())
    }
}

impl TryFrom<DescriptorCommand> for Query {
    type Error = NetMDError;

    fn try_from(value: DescriptorCommand) -> std::result::Result<Self, Self::Error> {
        Query::from_raw(&format!(
            "00 1808 {} {} 00",
            value.0.as_str(),
            value.1.as_str()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disc_title_open_read_frame() {
        let q: Query = DescriptorCommand(Descriptor::DiskTitleTd, DescriptorAction::OpenRead)
            .try_into()
            .unwrap();
        assert_eq!(q.0, [0x00, 0x18, 0x08, 0x10, 0x18, 0x01, 0x01, 0x00]);
    }

    #[test]
    fn audio_contents_close_frame() {
        let q: Query = DescriptorCommand(Descriptor::AudioContentsTd, DescriptorAction::Close)
            .try_into()
            .unwrap();
        assert_eq!(q.0, [0x00, 0x18, 0x08, 0x10, 0x10, 0x01, 0x00, 0x00]);
    }

    #[test]
    fn operating_status_block_uses_8000() {
        let q: Query =
            DescriptorCommand(Descriptor::OperatingStatusBlock, DescriptorAction::OpenRead)
                .try_into()
                .unwrap();
        assert_eq!(q.0, [0x00, 0x18, 0x08, 0x80, 0x00, 0x01, 0x00]);
    }

    #[test]
    fn root_open_write_frame() {
        let q: Query = DescriptorCommand(Descriptor::RootTd, DescriptorAction::OpenWrite)
            .try_into()
            .unwrap();
        assert_eq!(q.0, [0x00, 0x18, 0x08, 0x10, 0x10, 0x00, 0x03, 0x00]);
    }
}
