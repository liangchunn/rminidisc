//! Low-level USB transport: sending queries and reading protocol replies.
//!
//! Implements the NetMD control message exchange over libusb control/bulk
//! transfers, including interim-reply polling and retries. Higher-level command
//! modules build [`crate::query::Query`] values and parse the responses with
//! [`crate::scan`].

use std::thread::sleep;
use std::time::Duration;

use log::trace;
use rusb::request_type;

use crate::{
    error::{NetMDError, Result},
    query::Query,
    types::{
        ProtocolReply, ReadRequestData, ReadRequestHeader, INTERIM_RETRY_INTERVAL_MS,
        MAX_INTERIM_ATTEMPTS, READ_REPLY_POLL_INTERVAL_MS, USB_TIMEOUT_MILLIS,
    },
};

use super::NetMD;

/// USB bulk OUT endpoint address for track data. WebUSB endpoint `0x02`
/// (`netmd.ts:6`) maps to rusb endpoint address `0x02`.
pub(crate) const BULK_WRITE_ENDPOINT: u8 = 0x02;

/// Maximum bytes per single bulk-OUT libusb call.
///
/// NOTE: this is a deliberate deviation from the JS reference. `NetMD.writeBulk`
/// (`netmd.ts:231`) hands the entire packet to a single WebUSB `transferOut`
/// call and lets the browser split it into endpoint-sized USB transactions.
/// libusb/rusb does not do that splitting for us: a single multi-MB `write_bulk`
/// (the first SP packet is ~1 MB, and the whole payload can be ~79 MB) stalls on
/// some hosts (observed on macOS). We therefore split into `0x10000` pieces,
/// matching the chunk size `readBulk` uses for reads (`netmd-interface.ts:714`).
const BULK_WRITE_CHUNK: usize = 0x10000;

impl NetMD {
    pub(crate) fn send_query<M>(&self, message: M) -> Result<ReadRequestData>
    where
        M: TryInto<Query>,
        NetMDError: From<M::Error>,
    {
        self.send_query_ext(message, false)
    }

    /// Sends a command and reads the reply, performing protocol status checking.
    ///
    /// Mirrors `NetMDInterface.sendQuery` + `readReply`:
    /// - The command is sent once via control transfer (request `0x80`).
    /// - The reply is read; if the status byte is interim (`0x0f`) and
    ///   `accept_interim` is false, the read is retried with exponential backoff.
    /// - `0x08` maps to `NotImplemented`, `0x0a` to `Rejected`.
    pub(crate) fn send_query_ext<M>(
        &self,
        message: M,
        accept_interim: bool,
    ) -> Result<ReadRequestData>
    where
        M: TryInto<Query>,
        NetMDError: From<M::Error>,
    {
        let query: Query = message.try_into()?;
        trace!("  TX -> {:02x?}", query.0);
        self.handle.write_control(
            request_type(
                rusb::Direction::Out,
                rusb::RequestType::Vendor,
                rusb::Recipient::Interface,
            ),
            0x80,
            0,
            0,
            &query.0,
            Duration::from_millis(USB_TIMEOUT_MILLIS),
        )?;

        let reply = self.read_reply_checked(accept_interim)?;

        Ok(reply)
    }

    /// Reads a reply, checking the protocol status byte and retrying on interim.
    ///
    /// The status byte is the first byte of the reply payload. On success the
    /// status byte is left in place (callers strip it via scan `%?` templates).
    fn read_reply_checked(&self, accept_interim: bool) -> Result<ReadRequestData> {
        let mut attempt: u32 = 0;
        while attempt < MAX_INTERIM_ATTEMPTS {
            let data = self.read_reply()?;
            let status_byte = data
                .0
                .first()
                .copied()
                .ok_or(NetMDError::UnknownStatus(0))?;
            let status: ProtocolReply = status_byte.into();

            match status {
                ProtocolReply::NotImplemented => return Err(NetMDError::NotImplemented),
                ProtocolReply::Rejected => {
                    let hex = data
                        .0
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<String>();
                    return Err(NetMDError::Rejected(hex));
                }
                ProtocolReply::Interim if !accept_interim => {
                    let factor = (1u64 << attempt) - 1;
                    sleep(Duration::from_millis(INTERIM_RETRY_INTERVAL_MS * factor));
                    attempt += 1;
                    continue;
                }
                ProtocolReply::Accepted
                | ProtocolReply::Implemented
                | ProtocolReply::Changed
                | ProtocolReply::Interim => {
                    return Ok(data);
                }
                _ => return Err(NetMDError::UnknownStatus(status_byte)),
            }
        }
        Err(NetMDError::MaxInterimAttempts)
    }

    /// Reads the reply length header (request `0x01`). The third byte holds the
    /// payload length. Mirrors `NetMD.getReplyLength`.
    pub(crate) fn read_reply_length(&self) -> std::result::Result<ReadRequestHeader, rusb::Error> {
        let mut reply_header = ReadRequestHeader::new();
        self.handle.read_control(
            request_type(
                rusb::Direction::In,
                rusb::RequestType::Vendor,
                rusb::Recipient::Interface,
            ),
            0x01,
            0,
            0,
            &mut reply_header.0,
            Duration::from_millis(USB_TIMEOUT_MILLIS),
        )?;
        trace!("  RX <- {:02x?}", reply_header.0);
        Ok(reply_header)
    }

    pub(crate) fn read_reply(&self) -> std::result::Result<ReadRequestData, rusb::Error> {
        let mut reply_header = self.read_reply_length()?;
        let mut i: u32 = 0;
        while reply_header.is_empty() {
            sleep(Duration::from_millis(READ_REPLY_POLL_INTERVAL_MS << i));
            reply_header = self.read_reply_length()?;
            i += 1;
        }

        let mut reply = ReadRequestData::new(reply_header.len());

        self.handle.read_control(
            request_type(
                rusb::Direction::In,
                rusb::RequestType::Vendor,
                rusb::Recipient::Interface,
            ),
            0x81,
            0,
            0,
            &mut reply.0,
            Duration::from_millis(USB_TIMEOUT_MILLIS),
        )?;

        trace!("  RX <- {:02x?}", reply.0);

        let _ = self.read_reply_length();

        Ok(reply)
    }

    /// Reads a reply after a long-running bulk transfer (track commit).
    ///
    /// Unlike [`read_reply`], the reply-length poll here tolerates USB timeouts:
    /// the device can take several seconds to finalize the track before it produces
    /// a reply, during which the control-IN length read may time out. Each timeout
    /// is treated as "not ready yet" and retried up to an overall budget.
    pub(crate) fn read_reply_after_bulk(&self) -> Result<ReadRequestData> {
        const MAX_POLLS: u32 = 200;
        let mut polls = 0u32;
        let header = loop {
            match self.read_reply_length() {
                Ok(h) if !h.is_empty() => break h,
                Ok(_) => {}
                Err(rusb::Error::Timeout) => {}
                Err(e) => return Err(NetMDError::Usb(e)),
            }
            polls += 1;
            if polls >= MAX_POLLS {
                return Err(NetMDError::Usb(rusb::Error::Timeout));
            }
            sleep(Duration::from_millis(200));
        };

        let mut reply = ReadRequestData::new(header.len());
        self.handle.read_control(
            request_type(
                rusb::Direction::In,
                rusb::RequestType::Vendor,
                rusb::Recipient::Interface,
            ),
            0x81,
            0,
            0,
            &mut reply.0,
            Duration::from_millis(USB_TIMEOUT_MILLIS),
        )?;
        trace!("  RX <- {:02x?}", reply.0);
        Ok(reply)
    }

    /// Writes data to the bulk OUT endpoint. Mirrors `NetMD.writeBulk` (`netmd.ts:231`),
    /// except it splits the write into `BULK_WRITE_CHUNK`-sized libusb calls (see
    /// the constant's docs) so large SP payloads transfer reliably.
    pub(crate) fn write_bulk(&self, data: &[u8]) -> Result<()> {
        let mut written = 0;
        while written < data.len() {
            let end = (written + BULK_WRITE_CHUNK).min(data.len());
            let chunk = &data[written..end];
            let mut off = 0;
            while off < chunk.len() {
                let n = self.handle.write_bulk(
                    BULK_WRITE_ENDPOINT,
                    &chunk[off..],
                    Duration::from_millis(USB_TIMEOUT_MILLIS * 20),
                )?;
                if n == 0 {
                    return Err(NetMDError::BulkWriteNoProgress);
                }
                off += n;
            }
            written += chunk.len();
        }
        Ok(())
    }
}
