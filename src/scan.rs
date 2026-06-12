use anyhow::anyhow;

pub fn scan<'a>(template: &'a str, data: &'a [u8]) -> anyhow::Result<Vec<&'a [u8]>> {
    let mut index = 0;
    let mut buf = String::new();
    let mut result = vec![];
    for char in template.chars() {
        if buf.len() < 2 {
            if char.is_whitespace() {
                continue;
            }
            if !matches!(char, 'a'..='z' | 'A'..='Z' | '0'..='9' | '%' | '?' | '*') {
                anyhow::bail!("invalid character '{}'", char)
            }
            buf.push(char);
        }
        if buf.len() == 2 {
            if buf.starts_with('%') {
                let specifier =
                    (*buf.as_bytes().get(1).ok_or(anyhow!("missing specifier"))?) as char;
                match specifier {
                    '?' => {
                        index += 1;
                    }
                    'b' => {
                        let slice = data.get(index..index + 1).ok_or(anyhow!("out of bounds"))?;
                        result.push(slice);
                        index += 1;
                    }
                    'w' => {
                        let slice = data.get(index..index + 2).ok_or(anyhow!("out of bounds"))?;
                        result.push(slice);
                        index += 2;
                    }
                    'd' => {
                        let slice = data.get(index..index + 4).ok_or(anyhow!("out of bounds"))?;
                        result.push(slice);
                        index += 4;
                    }
                    'q' => {
                        let slice = data.get(index..index + 8).ok_or(anyhow!("out of bounds"))?;
                        result.push(slice);
                        index += 8;
                    }
                    '*' => {
                        let slice = data
                            .get(index..data.len())
                            .ok_or(anyhow!("out of bounds"))?;
                        result.push(slice);
                        index += data.len() - index;
                    }
                    _ => anyhow::bail!(format!("invalid format character {}", specifier)),
                }
            } else {
                let num = u8::from_str_radix(&buf, 16)?;
                let compare = data.get(index).ok_or(anyhow!(format!(
                    "format string contains '0x{num:02x}', but data buffer does not have this value"
                )))?;
                if num != *compare {
                    anyhow::bail!(format!("expected {compare}, got {num}"));
                }

                index += 1;
            }
            // clear the string buffer
            buf = String::new();
        }
    }
    if !buf.is_empty() {
        anyhow::bail!("invalid format, unmatched character '{}'", buf)
    }
    if index != data.len() {
        anyhow::bail!("data buffer contains unparsed residual data")
    }
    Ok(result)
}

#[test]
fn tt() {
    let z: u16 = 10; // 2 bytes
    println!("{:02x?}", z.to_le_bytes());
}

#[test]
fn t() {
    let input = "00 01 ff %b ff %w ff %d ff %q ff %? %b aa %*";
    let data = &[
        0x00, 0x01, 0xff, 0x01, 0xff, 0x01, 0x02, 0xff, 0x01, 0x02, 0x03, 0x04, 0xff, 0x01, 0x02,
        0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0xff, 0x0a, 0xff, 0xaa, 0x01, 0x02, 0x03, 0x04,
    ];

    let r = scan(input, data).unwrap();
    println!("{:?}", r);
}
