//! Building protocol query byte strings.
//!
//! [`QueryBuilder`] assembles a NetMD control message from raw hex fragments and
//! typed fields (`u8`/`u16`/...) into a [`Query`]. The hex strings include the
//! leading `00` status byte (the JS port prepends it at send time). See
//! `PORTING_REFERENCE.md` for the template language.

use crate::error::{NetMDError, Result};

pub(crate) struct Query(pub(crate) Vec<u8>);

impl From<Query> for Vec<u8> {
    fn from(val: Query) -> Self {
        val.0
    }
}

/// Programmatic query builder mirroring `formatQuery` (`query-utils.ts:23`).
///
/// The JS reference uses a printf-style format string with `%b`/`%w`/`%d`/`%*`
/// substitutions interleaved with hex literals. In Rust we expose a small typed
/// builder instead — each method appends bytes in big-endian order, matching the
/// `FORMAT_TYPE_LEN_DICT` behaviour. Use [`QueryBuilder::raw`] for the static hex
/// portions of a command.
#[derive(Debug, Clone, Default)]
pub(crate) struct QueryBuilder(Vec<u8>);

impl QueryBuilder {
    pub(crate) fn new() -> Self {
        QueryBuilder(Vec::new())
    }

    /// Appends static bytes parsed from a hex string (whitespace ignored).
    /// Mirrors the literal hex pairs in a `formatQuery` format string.
    pub(crate) fn raw(mut self, hex: &str) -> Result<Self> {
        let q = Query::from_raw(hex)?;
        self.0.extend_from_slice(&q.0);
        Ok(self)
    }

    /// Appends a single byte (`%b`).
    pub(crate) fn u8(mut self, value: u8) -> Self {
        self.0.push(value);
        self
    }

    /// Appends a big-endian u16 (`%w`).
    pub(crate) fn u16(mut self, value: u16) -> Self {
        self.0.extend_from_slice(&value.to_be_bytes());
        self
    }

    /// Appends a big-endian u32 (`%d`).
    pub(crate) fn u32(mut self, value: u32) -> Self {
        self.0.extend_from_slice(&value.to_be_bytes());
        self
    }

    /// Appends raw bytes verbatim (`%*`).
    pub(crate) fn bytes(mut self, value: &[u8]) -> Self {
        self.0.extend_from_slice(value);
        self
    }

    pub(crate) fn build(self) -> Query {
        Query(self.0)
    }
}

impl From<QueryBuilder> for Query {
    fn from(b: QueryBuilder) -> Self {
        b.build()
    }
}

impl Query {
    pub(crate) fn from_raw(value: &str) -> Result<Query> {
        let new_string = value
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>();
        if new_string.len() % 2 != 0 {
            return Err(NetMDError::InvalidQuery(
                "invalid command length".to_string(),
            ));
        }
        let buf = new_string
            .chars()
            .collect::<Vec<char>>()
            .chunks(2)
            .map(|c| c.iter().collect::<String>())
            .map(|bytes| {
                u8::from_str_radix(&bytes, 16).map_err(|source| {
                    NetMDError::InvalidQuery(format!("invalid hex byte {bytes}: {source}"))
                })
            })
            .collect::<Result<Vec<u8>>>()?;

        // insert 0x00 pad byte
        // buf.insert(0, 0x00);

        Ok(Query(buf))
    }
}

impl TryFrom<&str> for Query {
    type Error = NetMDError;

    fn try_from(value: &str) -> std::result::Result<Self, Self::Error> {
        Query::from_raw(value)
    }
}

impl TryFrom<String> for Query {
    type Error = NetMDError;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        Query::from_raw(&value)
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

    #[test]
    fn builder_matches_format_query() {
        use super::QueryBuilder;
        // Equivalent of formatQuery('1800 080046 f0030103 23 ff 1001 %w', track)
        let q: super::Query = QueryBuilder::new()
            .raw("1800 080046 f0030103 23 ff 1001")
            .unwrap()
            .u16(0x0005)
            .into();
        assert_eq!(
            q.0,
            [
                0x18, 0x00, 0x08, 0x00, 0x46, 0xf0, 0x03, 0x01, 0x03, 0x23, 0xff, 0x10, 0x01, 0x00,
                0x05
            ]
        );
    }

    #[test]
    fn builder_binary_substitution() {
        use super::QueryBuilder;
        // %b %d %* ordering, big-endian.
        let q: super::Query = QueryBuilder::new()
            .u8(0x12)
            .u32(0x01020304)
            .bytes(&[0xaa, 0xbb])
            .into();
        assert_eq!(q.0, [0x12, 0x01, 0x02, 0x03, 0x04, 0xaa, 0xbb]);
    }
}
