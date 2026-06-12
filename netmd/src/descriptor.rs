use crate::query::Query;

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
    fn to_str(&self) -> &str {
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
    fn to_str(&self) -> &str {
        match self {
            DescriptorAction::OpenRead => "01",
            DescriptorAction::OpenWrite => "03",
            DescriptorAction::Close => "00",
        }
    }
}

pub struct DescriptorCommand(pub Descriptor, pub DescriptorAction);

impl TryFrom<DescriptorCommand> for Query {
    type Error = anyhow::Error;

    fn try_from(value: DescriptorCommand) -> Result<Self, Self::Error> {
        Query::from_raw(&format!(
            "00 1808 {} {} 00",
            value.0.to_str(),
            value.1.to_str()
        ))
    }
}
