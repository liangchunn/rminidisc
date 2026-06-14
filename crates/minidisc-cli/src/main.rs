//! `minidisc-cli` — thin runner over the `netmd` library.
//!
//! Device interaction is delegated to the `netmd` crate; this binary only does
//! argument parsing and host-side audio preparation (ffmpeg / ATRAC encoding).

mod logbuf;
mod tui;

use std::path::Path;

use anyhow::{bail, Context};
use clap::{Args, Parser, Subcommand};
use lofty::prelude::*;
use lofty::read_from_path;
use log::info;
use netmd::track::MdTrack;
use netmd::wav;
use netmd::Wireformat;
use rusb::UsbContext;

/// NetMD MiniDisc command-line tool.
///
/// Dump metadata, upload/erase/rename/reorder tracks, and control playback
/// on NetMD MiniDisc devices.
#[derive(Parser)]
#[command(
    name = "minidisc-cli",
    about = "NetMD MiniDisc command-line tool",
    long_about = "Dump metadata, upload/erase/rename/reorder tracks, and control \
                  playback on NetMD MiniDisc devices."
)]
struct Cli {
    /// NetMD device as VID:PID (e.g. 054c:0084)
    #[arg(
        short = 'd',
        long,
        global = true,
        value_parser = |s: &str| parse_device_selector(s).map_err(|e| e.to_string())
    )]
    device: Option<netmd::DeviceSelector>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Dump disc + track metadata (default when no command is given)
    Info,
    /// Encode and write a track to the device
    Upload(UploadArgs),
    /// Erase a single track or the whole disc
    Erase {
        /// Track number (1-based, as shown by `info`) or "disc"
        target: Option<String>,
    },
    /// Set a track or disc title
    Rename {
        /// "disc" or a track number (1-based)
        target: String,
        /// New title
        title: String,
        /// Write the full-width title field
        #[arg(long)]
        full: bool,
    },
    /// Reorder a track
    #[command(name = "move")]
    MoveTrack {
        /// Source track (1-based)
        from: u16,
        /// Destination position (1-based)
        to: u16,
    },
    /// Start playback
    Play,
    /// Pause playback
    Pause,
    /// Stop playback
    Stop,
    /// Skip to the next track
    Next,
    /// Go back to the previous track
    Prev,
    /// Fast-forward
    Ff,
    /// Rewind
    Rewind,
    /// Eject the disc
    Eject,
    /// Seek to the start of a track
    Goto {
        /// Track number (1-based)
        track: u16,
    },
    /// Print a one-shot playback status snapshot
    Status,
    /// List supported NetMD devices
    List,
    /// Launch the interactive playback TUI
    Control,
    /// List the disc's group structure and per-track details
    Groups,
    /// Edit the disc's group structure
    Group {
        #[command(subcommand)]
        action: GroupAction,
    },
}

#[derive(Subcommand)]
enum GroupAction {
    /// Create a group over a 1-based track range (e.g. "1-3" or "5")
    Add {
        /// Track range, 1-based: "1-3" for a span or "5" for a single track
        range: String,
        /// Half-width group title
        name: String,
        /// Full-width group title
        #[arg(long)]
        full: Option<String>,
    },
    /// Rename a group by its index (as shown by `groups`)
    Rename {
        /// Group index from `groups`
        index: usize,
        /// New half-width group title
        name: String,
        /// New full-width group title
        #[arg(long)]
        full: Option<String>,
    },
    /// Dissolve a group by its index (as shown by `groups`)
    Remove {
        /// Group index from `groups`
        index: usize,
    },
}

#[derive(Args)]
struct UploadArgs {
    /// Input audio file
    #[arg(conflicts_with = "folder")]
    file: Option<String>,

    /// Upload every file in a directory (top-level only)
    #[arg(short = 'F', long, conflicts_with_all = ["file", "title"])]
    folder: Option<String>,

    /// Encoding format (sp, lp2, lp105, lp4)
    #[arg(short = 'f', long, default_value = "sp")]
    format: String,

    /// Track title (overrides file metadata tags; cannot be used with --folder)
    #[arg(short = 't', long, conflicts_with = "folder")]
    title: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // The interactive TUI installs its own in-memory logger (env_logger would
    // write to stderr and corrupt the full-screen UI). Every other command uses
    // the normal stderr logger.
    if matches!(cli.command, Some(Command::Control)) {
        return tui::run(cli.device);
    }
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    match cli.command.unwrap_or(Command::Info) {
        Command::Info => cmd_info(cli.device),
        Command::Upload(args) => cmd_upload(args, cli.device),
        Command::Erase { target } => cmd_erase(target, cli.device),
        Command::Rename {
            target,
            title,
            full,
        } => cmd_rename(target, title, full, cli.device),
        Command::MoveTrack { from, to } => cmd_move(from, to, cli.device),
        Command::Play => cmd_transport("play", cli.device),
        Command::Pause => cmd_transport("pause", cli.device),
        Command::Stop => cmd_transport("stop", cli.device),
        Command::Next => cmd_transport("next", cli.device),
        Command::Prev => cmd_transport("prev", cli.device),
        Command::Ff => cmd_transport("ff", cli.device),
        Command::Rewind => cmd_transport("rewind", cli.device),
        Command::Eject => cmd_transport("eject", cli.device),
        Command::Goto { track } => cmd_goto(track, cli.device),
        Command::Status => cmd_status(cli.device),
        Command::List => cmd_list(),
        Command::Groups => cmd_groups(cli.device),
        Command::Group { action } => cmd_group(action, cli.device),
        Command::Control => unreachable!("Control handled before env_logger init"),
    }
}

fn parse_device_selector(value: &str) -> anyhow::Result<netmd::DeviceSelector> {
    let (vendor, product) = value
        .split_once(':')
        .context("--device must be VID:PID, for example 054c:0084")?;
    let vendor_id = parse_hex_u16(vendor).context("invalid device vendor ID")?;
    let product_id = parse_hex_u16(product).context("invalid device product ID")?;

    if netmd::supported_device(vendor_id, product_id).is_none() {
        bail!("unsupported NetMD device {vendor_id:04x}:{product_id:04x}");
    }

    Ok(netmd::DeviceSelector::new(vendor_id, product_id))
}

fn parse_hex_u16(value: &str) -> anyhow::Result<u16> {
    let value = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    Ok(u16::from_str_radix(value, 16)?)
}

/// Parses a 1-based track index (as shown by `info`) into a 0-based `u16`.
fn parse_track_index(s: &str) -> anyhow::Result<u16> {
    let n: u32 = s
        .parse()
        .with_context(|| format!("invalid track number {s:?}"))?;
    if n == 0 {
        bail!("track numbers are 1-based (as shown by `info`)");
    }
    u16::try_from(n - 1).context("track number out of range")
}

fn cmd_list() -> anyhow::Result<()> {
    let devices = netmd::list_connected_devices()?;
    if devices.is_empty() {
        bail!("no supported NetMD devices connected");
    }
    for device in &devices {
        info!(
            "{:04x}:{:04x}  {}",
            device.vendor_id, device.product_id, device.name
        );
    }
    Ok(())
}

fn cmd_info(device: Option<netmd::DeviceSelector>) -> anyhow::Result<()> {
    let handle = netmd::open_device_matching(device)?;

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
    info!(
        "disc flags: raw=0x{:02x} writable={} write_protected={} unknown=0x{:02x}",
        disc_flags.raw(),
        disc_flags.is_writable(),
        disc_flags.is_write_protected(),
        disc_flags.unknown_bits()
    );

    match netmd::get_disc_capacity(&handle) {
        Ok(cap) => info!(
            "disc capacity: recorded={:?} total={:?} available={:?}",
            cap[0], cap[1], cap[2]
        ),
        Err(e) => info!("disc capacity unavailable: {e}"),
    }

    match netmd::get_full_operating_status(&handle) {
        Ok(status) => info!(
            "operating status: mode=0x{:02x} status={:?}",
            status.mode, status.status
        ),
        Err(e) => info!("operating status unavailable: {e}"),
    }

    let track_count = netmd::get_track_count(&handle)?;
    info!("track count: {track_count}");

    let group_list = netmd::get_track_group_list(&handle)?;
    let has_named_groups = group_list.iter().any(|g| g.name.is_some());

    for group in &group_list {
        if has_named_groups {
            match &group.name {
                Some(name) => info!("group: {name:?}"),
                None => info!("(ungrouped)"),
            }
        }
        for &track in &group.tracks {
            let title = netmd::get_track_title(&handle, track, false).unwrap_or_default();
            let length = netmd::get_track_length(&handle, track)?;
            let (encoding, channels) = netmd::get_track_encoding(&handle, track)?;
            let flags = netmd::get_track_flags(&handle, track)?;
            info!(
                "  track {}: {:?} len={:02}:{:02}:{:02}+{:03} enc={:?} ch={:?} flags=0x{:02x}",
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
    }

    netmd::close_device(&handle)?;
    Ok(())
}

fn cmd_erase(target: Option<String>, device: Option<netmd::DeviceSelector>) -> anyhow::Result<()> {
    let handle = netmd::open_device_matching(device)?;
    match target.as_deref() {
        None | Some("disc") => {
            netmd::erase_disc(&handle)?;
            info!("disc erased");
        }
        Some(s) => {
            let track = parse_track_index(s)?;
            netmd::erase_track(&handle, track)?;
            info!("erased track #{}", track + 1);
        }
    }
    netmd::close_device(&handle)?;
    Ok(())
}

fn cmd_rename(
    target: String,
    title: String,
    full: bool,
    device: Option<netmd::DeviceSelector>,
) -> anyhow::Result<()> {
    let handle = netmd::open_device_matching(device)?;
    if target == "disc" {
        netmd::rename_disc(&handle, &title, full)?;
        info!(
            "renamed disc to {title:?}{}",
            if full { " (full-width)" } else { "" }
        );
    } else {
        let track = parse_track_index(&target)?;
        netmd::set_track_title(&handle, track, &title, full)?;
        info!(
            "renamed track #{} to {title:?}{}",
            track + 1,
            if full { " (full-width)" } else { "" }
        );
    }
    netmd::close_device(&handle)?;
    Ok(())
}

fn cmd_move(from: u16, to: u16, device: Option<netmd::DeviceSelector>) -> anyhow::Result<()> {
    let source = parse_track_index(&from.to_string())?;
    let dest = parse_track_index(&to.to_string())?;

    let handle = netmd::open_device_matching(device)?;
    netmd::move_track(&handle, source, dest)?;
    info!("moved track #{} -> #{}", source + 1, dest + 1);
    netmd::close_device(&handle)?;
    Ok(())
}

fn cmd_transport(action: &str, device: Option<netmd::DeviceSelector>) -> anyhow::Result<()> {
    let handle = netmd::open_device_matching(device)?;
    match action {
        "play" => netmd::play(&handle)?,
        "pause" => netmd::pause(&handle)?,
        "stop" => netmd::stop(&handle)?,
        "next" => netmd::next_track(&handle)?,
        "prev" => netmd::previous_track(&handle)?,
        "ff" => netmd::fast_forward(&handle)?,
        "rewind" => netmd::rewind(&handle)?,
        "eject" => netmd::eject_disc(&handle)?,
        other => bail!("unknown transport action {other:?}"),
    }
    info!("{action}");
    netmd::close_device(&handle)?;
    Ok(())
}

fn cmd_goto(track: u16, device: Option<netmd::DeviceSelector>) -> anyhow::Result<()> {
    let track = parse_track_index(&track.to_string())?;
    let handle = netmd::open_device_matching(device)?;
    let resulting = netmd::goto_track(&handle, track)?;
    info!("seeked to track #{}", resulting + 1);
    netmd::close_device(&handle)?;
    Ok(())
}

fn cmd_status(device: Option<netmd::DeviceSelector>) -> anyhow::Result<()> {
    let handle = netmd::open_device_matching(device)?;
    let status = netmd::get_device_status(&handle)?;
    info!("disc present: {}", status.disc_present);
    info!("state: {:?}", status.state);
    match status.track {
        Some(t) => info!("track: #{}", t + 1),
        None => info!("track: -"),
    }
    match status.time {
        Some(t) => info!("time: {:02}:{:02}+{:03}", t.minute, t.second, t.frame),
        None => info!("time: -"),
    }
    netmd::close_device(&handle)?;
    Ok(())
}

fn cmd_groups(device: Option<netmd::DeviceSelector>) -> anyhow::Result<()> {
    let handle = netmd::open_device_matching(device)?;
    let disc = netmd::list_content(&handle)?;

    info!("disc title: {:?}", disc.title);
    if !disc.full_width_title.is_empty() {
        info!("disc full-width title: {:?}", disc.full_width_title);
    }
    info!(
        "writable={} write_protected={} tracks={}",
        disc.writable, disc.write_protected, disc.track_count
    );
    info!(
        "capacity: used={} total={} free={}",
        netmd::format_time_from_frames(disc.used),
        netmd::format_time_from_frames(disc.total),
        netmd::format_time_from_frames(disc.left),
    );

    let remaining = netmd::remaining_characters_for_titles(&disc, true);
    info!(
        "title space left: half-width={} chars full-width={} chars",
        remaining.half_width, remaining.full_width
    );

    for group in &disc.groups {
        match &group.title {
            Some(name) => {
                let full = group
                    .full_width_title
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .map(|s| format!(" / {s:?}"))
                    .unwrap_or_default();
                info!("[{}] group {:?}{}", group.index, name, full);
            }
            None => info!("[{}] (ungrouped)", group.index),
        }
        for track in &group.tracks {
            info!(
                "      track {}: {:?} {} enc={:?} ch={:?}{}",
                track.index + 1,
                track.title.as_deref().unwrap_or(""),
                netmd::format_time_from_frames(track.duration_frames),
                track.encoding,
                track.channel,
                if track.protected == netmd::TrackFlag::Protected {
                    " [protected]"
                } else {
                    ""
                },
            );
        }
    }

    netmd::close_device(&handle)?;
    Ok(())
}

/// Parses a 1-based track range (`"1-3"` or `"5"`) into a 0-based inclusive
/// `(first, last)` pair.
fn parse_track_range(s: &str) -> anyhow::Result<(u16, u16)> {
    let parse_one = |part: &str| -> anyhow::Result<u16> {
        let n: u32 = part
            .trim()
            .parse()
            .with_context(|| format!("invalid track number {part:?}"))?;
        if n == 0 {
            bail!("track numbers are 1-based (as shown by `info`)");
        }
        u16::try_from(n - 1).context("track number out of range")
    };

    match s.split_once('-') {
        Some((lo, hi)) => {
            let (first, last) = (parse_one(lo)?, parse_one(hi)?);
            if first > last {
                bail!("range start must not exceed end: {s:?}");
            }
            Ok((first, last))
        }
        None => {
            let only = parse_one(s)?;
            Ok((only, only))
        }
    }
}

fn cmd_group(action: GroupAction, device: Option<netmd::DeviceSelector>) -> anyhow::Result<()> {
    let handle = netmd::open_device_matching(device)?;
    let mut disc = netmd::list_content(&handle)?;

    match action {
        GroupAction::Add { range, name, full } => {
            let (first, last) = parse_track_range(&range)?;
            disc.add_group(first, last, name.clone(), full)?;
            netmd::rewrite_disc_groups(&handle, &disc)?;
            info!("created group {name:?} over tracks {range}");
        }
        GroupAction::Rename { index, name, full } => {
            disc.rename_group(index, name.clone(), full)?;
            netmd::rewrite_disc_groups(&handle, &disc)?;
            info!("renamed group [{index}] to {name:?}");
        }
        GroupAction::Remove { index } => {
            disc.remove_group(index)?;
            netmd::rewrite_disc_groups(&handle, &disc)?;
            info!("removed group [{index}]");
        }
    }

    netmd::close_device(&handle)?;
    Ok(())
}

fn resolve_title(file: &str) -> String {
    let tag_title = (|| -> Option<String> {
        let tagged_file = read_from_path(file).ok()?;
        let tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag())?;
        let title = tag.title()?;
        let trimmed = title.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })();

    match tag_title {
        Some(title) => {
            info!("title from metadata: {title:?} ({file})");
            title
        }
        None => {
            let fallback = Path::new(file)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("track")
                .to_string();
            info!("title from filename: {fallback:?} ({file})");
            fallback
        }
    }
}

fn upload_one<T: UsbContext>(
    handle: &rusb::DeviceHandle<T>,
    vendor: u16,
    product: u16,
    file: &str,
    requested: &str,
    title: &str,
) -> anyhow::Result<()> {
    info!("preparing {file:?} as {}", requested);
    let (wireformat, data) = prepare_audio(file, requested)?;
    info!(
        "prepared {} bytes, wire format {:?}",
        data.len(),
        wireformat
    );

    let track = MdTrack {
        title: title.to_string(),
        full_width_title: None,
        format: wireformat,
        data,
    };

    let mut last_pct = u64::MAX;
    let mut progress = |written: u64, total: u64| {
        let pct = written.saturating_mul(100).checked_div(total).unwrap_or(0);
        if pct != last_pct {
            info!("transferred {written} / {total} bytes ({pct}%)");
            last_pct = pct;
        }
    };

    let (track_num, uuid, ccid) =
        netmd::track::download_track(handle, &track, vendor, product, Some(&mut progress))?;

    info!("uploaded track #{track_num} (uuid={uuid} ccid={ccid}) title={title:?}");
    Ok(())
}

fn upload_folder<T: UsbContext>(
    handle: &rusb::DeviceHandle<T>,
    vendor: u16,
    product: u16,
    folder: &str,
    requested: &str,
) -> anyhow::Result<()> {
    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(folder)
        .with_context(|| format!("reading directory {folder:?}"))?
        .filter_map(|e| {
            let e = e.ok()?;
            let ft = e.file_type().ok()?;
            if ft.is_file() {
                Some(e.path())
            } else {
                None
            }
        })
        .collect();

    if paths.is_empty() {
        bail!("no files found in {folder:?}");
    }

    paths.sort();

    let mut succeeded: Vec<String> = Vec::new();
    let mut failed: Vec<(String, String)> = Vec::new();

    for path in &paths {
        let file = path.to_string_lossy();
        let title = resolve_title(&file);
        info!("uploading {file:?} as {title:?}");
        match upload_one(handle, vendor, product, &file, requested, &title) {
            Ok(()) => succeeded.push(file.into_owned()),
            Err(e) => {
                let err = format!("{e:#}");
                info!("FAILED {file:?}: {err}");
                failed.push((file.into_owned(), err));
            }
        }
    }

    info!("--- folder upload summary ---");
    info!("succeeded: {} track(s)", succeeded.len());
    info!("failed:    {} track(s)", failed.len());
    if !failed.is_empty() {
        info!("failures:");
        for (file, err) in &failed {
            info!("  {file}: {err}");
        }
    }

    if succeeded.is_empty() && !paths.is_empty() {
        bail!("all {} file(s) failed to upload", paths.len());
    }

    Ok(())
}

fn cmd_upload(args: UploadArgs, device: Option<netmd::DeviceSelector>) -> anyhow::Result<()> {
    if args.file.is_none() && args.folder.is_none() {
        bail!("upload requires an input file, or --folder for a directory");
    }

    let requested = args.format.to_lowercase();

    let handle = netmd::open_device_matching(device)?;
    let (vendor, product) = netmd::device_ids(&handle)?;

    let result = if let Some(file) = &args.file {
        let title = match &args.title {
            Some(t) => t.clone(),
            None => resolve_title(file),
        };
        upload_one(&handle, vendor, product, file, &requested, &title)
    } else if let Some(folder) = &args.folder {
        upload_folder(&handle, vendor, product, folder, &requested)
    } else {
        unreachable!()
    };

    netmd::close_device(&handle)?;
    result
}

/// Prepares audio for upload, returning the wire format and raw payload.
///
/// - If the input is an ATRAC3 WAV, its payload is used directly (header
///   stripped) and the requested format is ignored (the file dictates it).
/// - SP: normalize to big-endian s16 PCM via `md_pcm`.
/// - LP2/LP105/LP4: normalize to a 44.1 kHz stereo WAV via `md_pcm`, encode
///   as ATRAC3, then strip the resulting ATRAC3 WAV header.
fn prepare_audio(input: &str, requested: &str) -> anyhow::Result<(Wireformat, Vec<u8>)> {
    let raw = std::fs::read(input).with_context(|| format!("reading {input}"))?;

    // Already ATRAC3? Use it directly.
    if let Some((fmt, payload)) = wav::atrac3_info(&raw) {
        info!("input is ATRAC3 ({fmt:?}); using payload directly");
        return Ok((fmt, payload.to_vec()));
    }

    match requested {
        "sp" => {
            let data = md_pcm::decode_to_s16be_44100_stereo(input)
                .with_context(|| format!("normalizing {input} to 44.1 kHz stereo s16be"))?;
            Ok((Wireformat::Pcm, data))
        }
        "lp2" | "lp105" | "lp4" => {
            let wav_data = md_pcm::decode_to_wav_44100_stereo(input)
                .with_context(|| format!("normalizing {input} to 44.1 kHz stereo WAV"))?;

            // Encode to ATRAC3 (RIFF container) using the local atracdenc crate.
            let (codec, bitrate, wf) = match requested {
                "lp2" => (atracdenc::Codec::Atrac3, 128, Wireformat::Lp2),
                "lp105" => (atracdenc::Codec::Atrac3, 102, Wireformat::L105kbps),
                "lp4" => (atracdenc::Codec::Atrac3Lp4, 64, Wireformat::Lp4),
                _ => unreachable!(),
            };
            let encoded = atracdenc::EncodeBuilder::new()
                .codec(codec)
                .container(atracdenc::Container::Riff)
                .input_bytes(wav_data)
                .at3_settings(atracdenc::At3Settings {
                    bitrate_kbps: Some(bitrate),
                    ..Default::default()
                })
                .run_to_vec()
                .context("encoding ATRAC3 with atracdenc crate")?;

            let (detected, payload) = wav::atrac3_info(&encoded)
                .context("ATRAC encoder output was not a recognizable ATRAC3 WAV")?;
            if detected != wf {
                info!("note: ATRAC encoder produced {detected:?}, requested {wf:?}");
            }
            Ok((detected, payload.to_vec()))
        }
        other => bail!("unknown format {other:?} (expected sp, lp2, lp105, lp4)"),
    }
}
