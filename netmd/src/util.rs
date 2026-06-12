use std::array::TryFromSliceError;

pub fn parse_u16(buf: &[u8]) -> Result<u16, TryFromSliceError> {
    Ok(u16::from_be_bytes(<[u8; 2]>::try_from(buf)?))
}

pub fn parse_u8(buf: &[u8]) -> Result<u8, TryFromSliceError> {
    Ok(u8::from_be_bytes(<[u8; 1]>::try_from(buf)?))
}

pub fn parse_u32(buf: &[u8]) -> Result<u32, TryFromSliceError> {
    Ok(u32::from_be_bytes(<[u8; 4]>::try_from(buf)?))
}

pub fn parse_string(buf: &[u8]) -> anyhow::Result<String> {
    let (title, _encoding, errors) = encoding_rs::SHIFT_JIS.decode(buf);

    if errors {
        anyhow::bail!("invalid SHIFT_JIS string")
    }

    Ok(title.to_string())
}

pub fn encode_to_sjis(utf8: &str) -> anyhow::Result<Vec<u8>> {
    let (encoded, _encoding, errors) = encoding_rs::SHIFT_JIS.encode(utf8);
    if errors {
        anyhow::bail!("invalid UTF-8 for SHIFT_JIS encoding")
    }
    Ok(encoded.into_owned())
}

pub fn get_length_after_sjis_encode(utf8: &str) -> anyhow::Result<usize> {
    Ok(encode_to_sjis(utf8)?.len())
}

pub fn bcd_to_int(mut bcd: u16) -> u16 {
    let mut value: u16 = 0;
    let mut nibble: u32 = 0;
    while bcd != 0 {
        let nibble_value = (bcd & 0x0f) as u16;
        bcd >>= 4;
        value += nibble_value * 10u16.pow(nibble);
        nibble += 1;
    }
    value
}

pub fn parse_bcd_u8(buf: &[u8]) -> Result<u8, TryFromSliceError> {
    let val = parse_u8(buf)?;
    Ok(bcd_to_int(val as u16) as u8)
}

pub fn parse_bcd_u16(buf: &[u8]) -> Result<u16, TryFromSliceError> {
    let val = parse_u16(buf)?;
    Ok(bcd_to_int(val))
}

pub fn format_time_from_frames(value: u32) -> String {
    let f = value % 512;
    let value = (value - f) / 512;
    let s = value % 60;
    let value = (value - s) / 60;
    let m = value % 60;
    let h = (value - m) / 60;
    format!("{:02}:{:02}:{:02}+{:03}", h, m, s, f)
}

pub fn time_to_frames(time: &[u32; 4]) -> u32 {
    ((time[0] * 60 + time[1]) * 60 + time[2]) * 512 + time[3]
}
