use std::array::TryFromSliceError;

pub fn parse_u16(buf: &[u8]) -> Result<u16, TryFromSliceError> {
    Ok(u16::from_be_bytes(<[u8; 2]>::try_from(buf)?))
}

pub fn parse_u8(buf: &[u8]) -> Result<u8, TryFromSliceError> {
    Ok(u8::from_be_bytes(<[u8; 1]>::try_from(buf)?))
}

pub fn parse_string(buf: &[u8]) -> anyhow::Result<String> {
    let (title, _encoding, errors) = encoding_rs::SHIFT_JIS.decode(buf);

    if errors {
        anyhow::bail!("invalid SHIFT_JIS string")
    }

    Ok(title.to_string())
}
