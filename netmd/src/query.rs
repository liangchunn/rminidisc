pub struct Query(pub Vec<u8>);

impl From<Query> for Vec<u8> {
    fn from(val: Query) -> Self {
        val.0
    }
}

impl Query {
    pub fn from_raw(value: &str) -> anyhow::Result<Query> {
        let new_string = value
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>();
        if new_string.len() % 2 != 0 {
            anyhow::bail!("invalid command length")
        }
        let buf = new_string
            .chars()
            .collect::<Vec<char>>()
            .chunks(2)
            .map(|c| c.iter().collect::<String>())
            .map(|bytes| u8::from_str_radix(&bytes, 16).unwrap())
            .collect::<Vec<u8>>();

        // insert 0x00 pad byte
        // buf.insert(0, 0x00);

        Ok(Query(buf))
    }
}

#[cfg(test)]
mod tests {
    use crate::query::Query;

    #[test]
    fn parse_from_string() {
        let command = format!(
            "00 1806 02201801 00{:02x} 3000 0a00 ff00 {:04x}{:04x}",
            0, 0, 0
        );
        let command: Query = Query::from_raw(&command).unwrap();
        assert_eq!(
            command.0,
            [
                0x00, 0x18, 0x06, 0x02, 0x20, 0x18, 0x01, 0x00, 0x00, 0x30, 0x00, 0x0a, 0x00, 0xff,
                0x00, 0x00, 0x00, 0x00, 0x00
            ]
        );
    }

    #[test]
    fn whitespace_is_ignored() {
        assert_eq!(Query::from_raw("18 06").unwrap().0, [0x18, 0x06]);
        assert_eq!(Query::from_raw("1806").unwrap().0, [0x18, 0x06]);
        assert_eq!(Query::from_raw("18  06  ff").unwrap().0, [0x18, 0x06, 0xff]);
    }

    #[test]
    fn odd_length_is_rejected() {
        // After stripping whitespace, an odd number of hex digits is invalid.
        assert!(Query::from_raw("180").is_err());
        assert!(Query::from_raw("18 0").is_err());
    }

    #[test]
    fn empty_is_valid_empty_buffer() {
        assert_eq!(Query::from_raw("").unwrap().0, Vec::<u8>::new());
    }
}

impl TryFrom<&str> for Query {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Query::from_raw(value)
    }
}

impl TryFrom<String> for Query {
    type Error = anyhow::Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Query::from_raw(&value)
    }
}
