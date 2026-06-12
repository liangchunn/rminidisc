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
            // Endianness override markers (`%<`, `%>`) immediately follow `%`.
            // Since `scan` yields raw byte slices (length is endianness-agnostic),
            // the marker is accepted and skipped; the caller decides interpretation.
            if (char == '<' || char == '>') && buf == "%" {
                continue;
            }
            if !matches!(char, 'a'..='z' | 'A'..='Z' | '0'..='9' | '%' | '?' | '*' | '#') {
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
                    'B' => {
                        let slice = data.get(index..index + 1).ok_or(anyhow!("out of bounds"))?;
                        result.push(slice);
                        index += 1;
                    }
                    'W' => {
                        let slice = data.get(index..index + 2).ok_or(anyhow!("out of bounds"))?;
                        result.push(slice);
                        index += 2;
                    }
                    'x' | 's' => {
                        let len_bytes =
                            data.get(index..index + 2).ok_or(anyhow!("out of bounds"))?;
                        let length = u16::from_be_bytes(<[u8; 2]>::try_from(len_bytes)?) as usize;
                        index += 2;
                        let slice = data
                            .get(index..index + length)
                            .ok_or(anyhow!("out of bounds"))?;
                        result.push(slice);
                        index += length;
                    }
                    'z' => {
                        let length = *data.get(index).ok_or(anyhow!("out of bounds"))? as usize;
                        index += 1;
                        let slice = data
                            .get(index..index + length)
                            .ok_or(anyhow!("out of bounds"))?;
                        result.push(slice);
                        index += length;
                    }
                    '*' | '#' => {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t() {
        let input = "00 01 ff %b ff %w ff %d ff %q ff %? %b aa %*";
        let data = &[
            0x00, 0x01, 0xff, 0x01, 0xff, 0x01, 0x02, 0xff, 0x01, 0x02, 0x03, 0x04, 0xff, 0x01,
            0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0xff, 0x0a, 0xff, 0xaa, 0x01, 0x02, 0x03,
            0x04,
        ];

        let r = scan(input, data).unwrap();
        println!("{:?}", r);
    }

    #[test]
    fn test_bcd_scan() {
        // template: 6 literal bytes (0001 0006 0000) + 4 BCD bytes
        let input = "0001 0006 0000 %B %B %B %B";
        let data = &[0x00, 0x01, 0x00, 0x06, 0x00, 0x00, 0x10, 0x20, 0x30, 0x40];
        let r = scan(input, data).unwrap();
        assert_eq!(r.len(), 4);
        // %B yields the raw byte slice; caller decodes via parse_bcd_u8
        assert_eq!(r[0], &[0x10]);
        assert_eq!(r[3], &[0x40]);
    }

    #[test]
    fn test_endianness_override_accepted() {
        // `%<w` / `%>d` markers must be parseable; slices are length-only.
        let input = "%<w %>d";
        let data = &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06];
        let r = scan(input, data).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0], &[0x01, 0x02]);
        assert_eq!(r[1], &[0x03, 0x04, 0x05, 0x06]);
    }

    #[test]
    fn test_length_prefixed_x() {
        let input = "%x";
        let data = &[0x00, 0x05, b'H', b'e', b'l', b'l', b'o'];
        let r = scan(input, data).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0], b"Hello");
    }

    #[test]
    fn test_length_prefixed_z() {
        let input = "%z";
        let data = &[0x05, b'H', b'e', b'l', b'l', b'o'];
        let r = scan(input, data).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0], b"Hello");
    }

    #[test]
    fn test_remaining() {
        let input = "%b %*";
        let data = &[0x01, 0x02, 0x03, 0x04];
        let r = scan(input, data).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[1].len(), 3);
    }
}
