//! `rmd` — thin runner over the `netmd` library.
//!
//! Currently dumps disc + track metadata read from the connected device. This
//! will be refactored into a TUI later; all device interaction is delegated to
//! the `netmd` crate.

use log::info;

fn main() -> anyhow::Result<()> {
    env_logger::init();

    let handle = netmd::open_device()?;

    let title = netmd::get_disc_title(&handle, false)?;
    info!("disc title: {title:?}");

    let full_title = netmd::get_disc_title(&handle, true)?;
    info!("disc full-width title: {full_title:?}");

    match netmd::get_disc_subunit_identifier(&handle) {
        Ok(level) => info!("netmd level: 0x{level:02x}"),
        Err(e) => info!("subunit identifier unavailable: {e}"),
    }

    let disc_present = netmd::is_disc_present(&handle)?;
    info!("disc present: {disc_present}");

    let disc_flags = netmd::get_disc_flags(&handle)?;
    info!("disc flags: 0x{disc_flags:02x}");

    match netmd::get_disc_capacity(&handle) {
        Ok(cap) => info!(
            "disc capacity: recorded={:?} total={:?} available={:?}",
            cap[0], cap[1], cap[2]
        ),
        Err(e) => info!("disc capacity unavailable: {e}"),
    }

    match netmd::get_full_operating_status(&handle) {
        Ok((mode, status)) => info!("operating status: mode={mode} status=0x{status:04x}"),
        Err(e) => info!("operating status unavailable: {e}"),
    }

    let track_count = netmd::get_track_count(&handle)?;
    info!("track count: {track_count}");

    for track in 0..track_count as u16 {
        let title = netmd::get_track_title(&handle, track, false).unwrap_or_default();
        let length = netmd::get_track_length(&handle, track)?;
        let (encoding, channels) = netmd::get_track_encoding(&handle, track)?;
        let flags = netmd::get_track_flags(&handle, track)?;
        info!(
            "track {}: {:?} len={:02}:{:02}:{:02}+{:03} enc=0x{:02x} ch={} flags=0x{:02x}",
            track + 1,
            title,
            length[0],
            length[1],
            length[2],
            length[3],
            encoding,
            channels,
            flags
        );
    }

    netmd::close_device(&handle)?;

    Ok(())
}
