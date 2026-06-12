use std::time::Duration;

use log::{debug, info, trace};
use rusb::{request_type, DeviceHandle, UsbContext};

use crate::{
    descriptor::{Descriptor, DescriptorAction, DescriptorCommand},
    query::Query,
    scan::scan,
    types::{ReadRequestData, ReadRequestHeader, USB_TIMEOUT_MILLIS},
    util::{parse_string, parse_u16, parse_u8},
};

mod descriptor;
mod query;
mod scan;
mod types;
mod util;

// TODO: to return concrete `.try_into()`` errors, use the return type:
// TODO:    Result<ReadRequestData, M::Error>
// TODO: and remove
// TODO:    anyhow::Error: From<M::Error>
fn send_query<T, M>(handle: &DeviceHandle<T>, message: M) -> anyhow::Result<ReadRequestData>
where
    T: UsbContext,
    M: TryInto<Query>,
    anyhow::Error: From<M::Error>,
{
    let query: Query = message.try_into()?;
    trace!("  TX → {:02x?}", query.0);
    handle.write_control(
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

    let reply = read_reply(handle)?;

    Ok(reply)
}

fn get_disk_title<T: UsbContext>(handle: &DeviceHandle<T>, w_char: bool) -> anyhow::Result<String> {
    let mut done = 0;
    let mut remaining = 0;
    let mut total = 1;
    let mut chunk_size: u16;

    let mut sink = vec![];

    let w_char: u8 = if w_char { 1 } else { 0 };

    debug!("get disk title");

    while done < total {
        let query = format!(
            "00 1806 02201801 00{:02x} 3000 0a00 ff00 {:04x}{:04x}",
            w_char, remaining, done
        );
        // let query = Query::from_raw(&query)?;
        let reply = send_query(handle, query)?;

        if remaining == 0 {
            let data = scan(
                "%? 1806 02201801 00%? 3000 0a00 1000 %w0000 %?%?000a %w %*",
                &reply.0,
            )?;

            if let [cz, t, d] = &data[..] {
                chunk_size = parse_u16(cz)?;
                total = parse_u16(t)?;
                sink.push(parse_string(d)?);
            } else {
                unreachable!()
            }
            chunk_size -= 6;
        } else {
            let data = reply.scan("%? 1806 02201801 00%? 3000 0a00 1000 %w%?%? %*")?;
            if let [cz, d] = &data[..] {
                chunk_size = parse_u16(cz)?;
                sink.push(parse_string(d)?);
            } else {
                unreachable!()
            }
        }
        done += chunk_size;
        remaining = total - done;
    }

    Ok(sink.join(""))
}

fn get_track_count<T: UsbContext>(handle: &DeviceHandle<T>) -> anyhow::Result<u8> {
    debug!("get track count");
    let reply = send_query(handle, "00 1806 02101001 3000 1000 ff00 00000000")?;

    let data = reply.scan("%? 1806 02101001 %?%? %?%? 1000 00%?0000 0006 0010000200%b")?;

    if let [tc] = &data[..] {
        let track_count = parse_u8(tc)?;

        Ok(track_count)
    } else {
        unreachable!()
    }
}

fn read_reply<T: UsbContext>(handle: &DeviceHandle<T>) -> Result<ReadRequestData, rusb::Error> {
    // header is 4 bytes, which the third byte is the length of the next message
    // let mut reply_header = [0; 4];

    let mut reply_header = ReadRequestHeader::new();

    handle.read_control(
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

    trace!("  RX ← {:02x?}", reply_header.0);

    let mut reply = ReadRequestData::new(reply_header.len());

    handle.read_control(
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

    trace!("  RX ← {:02x?}", reply.0);

    Ok(reply)
}

fn main() -> anyhow::Result<()> {
    env_logger::init();

    let device = rusb::devices()?
        .iter()
        .filter(|device| {
            let device_desc = device.device_descriptor().unwrap();
            device_desc.vendor_id() == 0x054c && device_desc.product_id() == 0x0084
        })
        .collect::<Vec<_>>();
    let device = device.first().expect("cannot find device");
    let device_desc = device.device_descriptor()?;
    let mut handle = device.open()?;
    let langs = handle.read_languages(Duration::from_secs(5))?;

    let manufacturer = langs
        .iter()
        .filter_map(|lang| {
            handle
                .read_manufacturer_string(*lang, &device_desc, Duration::from_secs(5))
                .ok()
        })
        .collect::<Vec<_>>();

    println!(
        "Bus {:03} Device {:03} ID {:04x}:{:04x}",
        device.bus_number(),
        device.address(),
        device_desc.vendor_id(),
        device_desc.product_id()
    );
    println!("Manufacturer: {}", manufacturer.join(", "));

    handle.claim_interface(0)?;

    // audioContentsTD openRead
    debug!("audio contents TD open read");
    send_query(
        &handle,
        DescriptorCommand(Descriptor::AudioContentsTd, DescriptorAction::OpenRead),
    )?;
    // diskTitleTD openRead
    debug!("disk title TD open read");
    send_query(
        &handle,
        DescriptorCommand(Descriptor::DiskTitleTd, DescriptorAction::OpenRead),
    )?;

    let title = get_disk_title(&handle, false)?;
    info!("title: {title}");

    // diskTitleTD close
    debug!("audio contents TD close");
    send_query(
        &handle,
        DescriptorCommand(Descriptor::DiskTitleTd, DescriptorAction::Close),
    )?;
    // audioContentsTD close
    debug!("disk title TD close");
    send_query(
        &handle,
        DescriptorCommand(Descriptor::AudioContentsTd, DescriptorAction::Close),
    )?;

    // audioContentsTD openRead
    debug!("audio contents TD open read");
    send_query(
        &handle,
        DescriptorCommand(Descriptor::AudioContentsTd, DescriptorAction::OpenRead),
    )?;

    let track_count = get_track_count(&handle)?;
    info!("track count: {track_count}");
    // audioContentsTD close
    debug!("audio contents TD close");
    send_query(
        &handle,
        DescriptorCommand(Descriptor::DiskTitleTd, DescriptorAction::Close),
    )?;

    handle.release_interface(0)?;

    Ok(())
}

// { vendorId: 0x054c, deviceId: 0x0084, name: 'Sony MZ-N505' },
