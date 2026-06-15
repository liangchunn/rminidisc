//! `rminidisc` — thin runner over the `netmd` library.
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
use netmd::NetMD;
use netmd::Wireformat;
use tempfile::NamedTempFile;

/// NetMD MiniDisc command-line tool.
///
/// Dump metadata, upload/erase/rename/reorder tracks, and control playback
/// on NetMD MiniDisc devices.
#[derive(Parser)]
#[command(
    name = "rminidisc",
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
    let netmd = netmd::open_device_matching(device)?;

    let title = netmd.get_disc_title(false)?;
    info!("disc title: {title:?}");

    let full_title = netmd.get_disc_title(true)?;
    info!("disc full-width title: {full_title:?}");

    match netmd.get_disc_subunit_identifier() {
        Ok(level) => info!("netmd level: 0x{level:02x}"),
        Err(e) => info!("subunit identifier unavailable: {e}"),
    }

    let disc_present = netmd.is_disc_present()?;
    info!("disc present: {disc_present}");

    let disc_flags = netmd.get_disc_flags()?;
    info!(
        "disc flags: raw=0x{:02x} writable={} write_protected={} unknown=0x{:02x}",
        disc_flags.raw(),
        disc_flags.is_writable(),
        disc_flags.is_write_protected(),
        disc_flags.unknown_bits()
    );

    match netmd.get_disc_capacity() {
        Ok(cap) => info!(
            "disc capacity: recorded={:?} total={:?} available={:?}",
            cap[0], cap[1], cap[2]
        ),
        Err(e) => info!("disc capacity unavailable: {e}"),
    }

    match netmd.get_full_operating_status() {
        Ok(status) => info!(
            "operating status: mode=0x{:02x} status={:?}",
            status.mode, status.status
        ),
        Err(e) => info!("operating status unavailable: {e}"),
    }

    let track_count = netmd.get_track_count()?;
    info!("track count: {track_count}");

    let group_list = netmd.get_track_group_list()?;
    let has_named_groups = group_list.iter().any(|g| g.name.is_some());

    for group in &group_list {
        if has_named_groups {
            match &group.name {
                Some(name) => info!("group: {name:?}"),
                None => info!("(ungrouped)"),
            }
        }
        for &track in &group.tracks {
            let title = netmd.get_track_title(track, false).unwrap_or_default();
            let length = netmd.get_track_length(track)?;
            let (encoding, channels) = netmd.get_track_encoding(track)?;
            let flags = netmd.get_track_flags(track)?;
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

    netmd.close()?;
    Ok(())
}

fn cmd_erase(target: Option<String>, device: Option<netmd::DeviceSelector>) -> anyhow::Result<()> {
    let netmd = netmd::open_device_matching(device)?;
    match target.as_deref() {
        None | Some("disc") => {
            netmd.erase_disc()?;
            info!("disc erased");
        }
        Some(s) => {
            let track = parse_track_index(s)?;
            netmd.erase_track(track)?;
            info!("erased track #{}", track + 1);
        }
    }
    netmd.close()?;
    Ok(())
}

fn cmd_rename(
    target: String,
    title: String,
    full: bool,
    device: Option<netmd::DeviceSelector>,
) -> anyhow::Result<()> {
    let netmd = netmd::open_device_matching(device)?;
    if target == "disc" {
        netmd.rename_disc(&title, full)?;
        info!(
            "renamed disc to {title:?}{}",
            if full { " (full-width)" } else { "" }
        );
    } else {
        let track = parse_track_index(&target)?;
        netmd.set_track_title(track, &title, full)?;
        info!(
            "renamed track #{} to {title:?}{}",
            track + 1,
            if full { " (full-width)" } else { "" }
        );
    }
    netmd.close()?;
    Ok(())
}

fn cmd_move(from: u16, to: u16, device: Option<netmd::DeviceSelector>) -> anyhow::Result<()> {
    let source = parse_track_index(&from.to_string())?;
    let dest = parse_track_index(&to.to_string())?;

    let netmd = netmd::open_device_matching(device)?;
    netmd.move_track(source, dest)?;
    info!("moved track #{} -> #{}", source + 1, dest + 1);
    netmd.close()?;
    Ok(())
}

fn cmd_transport(action: &str, device: Option<netmd::DeviceSelector>) -> anyhow::Result<()> {
    let netmd = netmd::open_device_matching(device)?;
    match action {
        "play" => netmd.play()?,
        "pause" => netmd.pause()?,
        "stop" => netmd.stop()?,
        "next" => netmd.next_track()?,
        "prev" => netmd.previous_track()?,
        "ff" => netmd.fast_forward()?,
        "rewind" => netmd.rewind()?,
        "eject" => netmd.eject_disc()?,
        other => bail!("unknown transport action {other:?}"),
    }
    info!("{action}");
    netmd.close()?;
    Ok(())
}

fn cmd_goto(track: u16, device: Option<netmd::DeviceSelector>) -> anyhow::Result<()> {
    let track = parse_track_index(&track.to_string())?;
    let netmd = netmd::open_device_matching(device)?;
    let resulting = netmd.goto_track(track)?;
    info!("seeked to track #{}", resulting + 1);
    netmd.close()?;
    Ok(())
}

fn cmd_status(device: Option<netmd::DeviceSelector>) -> anyhow::Result<()> {
    let netmd = netmd::open_device_matching(device)?;
    let status = netmd.get_device_status()?;
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
    netmd.close()?;
    Ok(())
}

fn cmd_groups(device: Option<netmd::DeviceSelector>) -> anyhow::Result<()> {
    let netmd = netmd::open_device_matching(device)?;
    let disc = netmd.list_content()?;

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

    let remaining = disc.remaining_title_chars(true);
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

    netmd.close()?;
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
    let netmd = netmd::open_device_matching(device)?;
    let mut disc = netmd.list_content()?;

    match action {
        GroupAction::Add { range, name, full } => {
            let (first, last) = parse_track_range(&range)?;
            disc.add_group(first, last, name.clone(), full)?;
            netmd.rewrite_disc_groups(&disc)?;
            info!("created group {name:?} over tracks {range}");
        }
        GroupAction::Rename { index, name, full } => {
            disc.rename_group(index, name.clone(), full)?;
            netmd.rewrite_disc_groups(&disc)?;
            info!("renamed group [{index}] to {name:?}");
        }
        GroupAction::Remove { index } => {
            disc.remove_group(index)?;
            netmd.rewrite_disc_groups(&disc)?;
            info!("removed group [{index}]");
        }
    }

    netmd.close()?;
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

fn upload_one(netmd: &NetMD, file: &str, requested: &str, title: &str) -> anyhow::Result<()> {
    info!("preparing {file:?} as {}", requested);
    let prepared = prepare_audio(file, requested)?;
    info!(
        "prepared {} bytes, wire format {:?}",
        prepared.source.len(),
        prepared.wireformat
    );

    let track = MdTrack {
        title: title.to_string(),
        full_width_title: None,
        format: prepared.wireformat,
        source: prepared.source,
    };

    let mut last_pct = u64::MAX;
    let mut progress = |written: u64, total: u64| {
        let pct = written.saturating_mul(100).checked_div(total).unwrap_or(0);
        if pct != last_pct {
            info!("transferred {written} / {total} bytes ({pct}%)");
            last_pct = pct;
        }
    };

    let (track_num, uuid, ccid) = netmd.download_track(&track, Some(&mut progress))?;

    info!("uploaded track #{track_num} (uuid={uuid} ccid={ccid}) title={title:?}");
    Ok(())
}

fn upload_folder(netmd: &NetMD, folder: &str, requested: &str) -> anyhow::Result<()> {
    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(folder)
        .with_context(|| format!("reading directory {folder:?}"))?
        .filter_map(|e| {
            let e = e.ok()?;
            let ft = e.file_type().ok()?;
            if ft.is_file() && md_pcm::probe_audio(&e.path()) {
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
        match upload_one(netmd, &file, requested, &title) {
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

    let netmd = netmd::open_device_matching(device)?;

    let result = if let Some(file) = &args.file {
        let title = match &args.title {
            Some(t) => t.clone(),
            None => resolve_title(file),
        };
        upload_one(&netmd, file, &requested, &title)
    } else if let Some(folder) = &args.folder {
        upload_folder(&netmd, folder, &requested)
    } else {
        unreachable!()
    };

    netmd.close()?;
    result
}

/// Reads only the leading bytes of `path` and reports whether it is an ATRAC3
/// WAV. Avoids loading large non-ATRAC3 files such as m4a fully into memory just
/// to inspect their header. The RIFF `fmt ` chunk is well within the first few
/// KiB for any real ATRAC3 WAV.
///
/// If the prefix is a RIFF/WAVE file whose `fmt ` chunk lies beyond the probed
/// bytes (e.g. behind a large JUNK/LIST/metadata chunk), the probe is
/// inconclusive and the caller must perform a full-file parse to be sure.
fn is_atrac3_file(path: &str) -> anyhow::Result<bool> {
    use std::io::Read;

    const HEADER_PROBE_BYTES: usize = 8 * 1024;
    let mut file = std::fs::File::open(path).with_context(|| format!("opening {path}"))?;
    let mut header = vec![0u8; HEADER_PROBE_BYTES];
    let mut filled = 0;
    while filled < header.len() {
        match file
            .read(&mut header[filled..])
            .with_context(|| format!("reading header of {path}"))?
        {
            0 => break,
            n => filled += n,
        }
    }
    header.truncate(filled);

    match wav::atrac3_format(&header) {
        wav::HeaderProbe::Found(fmt) => Ok(fmt.is_some()),
        wav::HeaderProbe::NotWav => Ok(false),
        // `fmt ` is past the prefix; fall back to a full parse. This re-reads the
        // file, but only for genuine RIFF/WAVE inputs with an unusually large
        // pre-`fmt ` chunk — not for the common m4a/large-non-WAV case.
        wav::HeaderProbe::Inconclusive => {
            let raw = std::fs::read(path).with_context(|| format!("reading {path}"))?;
            Ok(wav::atrac3_info(&raw).is_some())
        }
    }
}

/// Prepares audio for upload, returning the wire format and a streamable
/// payload source.
///
/// - If the input is an ATRAC3 WAV, its payload is used directly (header
///   stripped) and the requested format is ignored (the file dictates it).
/// - SP: normalize to big-endian s16 PCM via `md_pcm`, streamed to a temp file.
/// - LP2/LP105/LP4: normalize to a 44.1 kHz stereo WAV (temp file) via `md_pcm`,
///   encode as ATRAC3, then strip the resulting ATRAC3 WAV header.
///
/// Result of preparing an input file for upload: the wire format and a
/// streamable payload source, plus any temp files that must outlive the upload.
struct PreparedAudio {
    wireformat: Wireformat,
    source: netmd::TrackSource,
    /// Kept alive so the backing temp file (if any) is not deleted before the
    /// upload reads it; dropped (and removed) when `PreparedAudio` is dropped.
    _temp: Option<NamedTempFile>,
}

fn prepare_audio(input: &str, requested: &str) -> anyhow::Result<PreparedAudio> {
    // Cheaply probe the header first: a large non-ATRAC3 file (e.g. a 700MB
    // m4a) should not be slurped into memory just to discover it is not an
    // ATRAC3 WAV. The `fmt ` chunk lives near the start of the file.
    if is_atrac3_file(input)? {
        // Confirmed ATRAC3: read the whole file and use its payload directly.
        let raw = std::fs::read(input).with_context(|| format!("reading {input}"))?;
        let (fmt, payload) = wav::atrac3_info(&raw)
            .context("file looked like ATRAC3 from its header but failed full parse")?;
        info!("input is ATRAC3 ({fmt:?}); using payload directly");
        return Ok(PreparedAudio {
            wireformat: fmt,
            source: netmd::TrackSource::Memory(payload.to_vec()),
            _temp: None,
        });
    }

    match requested {
        "sp" => {
            // Stream s16be PCM straight to a temp file. SP payloads are large
            // (~10 MB/min) and are later stream-encrypted from disk, so the full
            // PCM never needs to be resident in memory.
            let mut pcm_tmp =
                NamedTempFile::with_prefix("minidisc_s16be_").context("creating temp PCM file")?;
            let len = {
                let mut writer = std::io::BufWriter::new(pcm_tmp.as_file_mut());
                md_pcm::decode_to_s16be_44100_stereo_writer(input, &mut writer)
                    .with_context(|| format!("normalizing {input} to 44.1 kHz stereo s16be"))?;
                std::io::Write::flush(&mut writer).ok();
                drop(writer);
                pcm_tmp
                    .as_file()
                    .metadata()
                    .with_context(|| format!("sizing temp PCM {}", pcm_tmp.path().display()))?
                    .len() as usize
            };
            Ok(PreparedAudio {
                wireformat: Wireformat::Pcm,
                source: netmd::TrackSource::File {
                    path: pcm_tmp.path().to_path_buf(),
                    len,
                },
                _temp: Some(pcm_tmp),
            })
        }
        "lp2" | "lp105" | "lp4" => {
            // Stream the normalized WAV straight to a temp file rather than
            // buffering the whole (potentially gigabyte-scale) PCM in memory.
            let mut wav_tmp =
                NamedTempFile::with_prefix("minidisc_wav_").context("creating temp WAV file")?;
            {
                let writer = std::io::BufWriter::new(wav_tmp.as_file_mut());
                md_pcm::decode_to_wav_44100_stereo_writer(input, writer)
                    .with_context(|| format!("normalizing {input} to 44.1 kHz stereo WAV"))?;
            }

            // Encode to ATRAC3 (RIFF container) using the local atracdenc crate.
            let (codec, bitrate, wf) = match requested {
                "lp2" => (atracdenc::Codec::Atrac3, 128, Wireformat::Lp2),
                "lp105" => (atracdenc::Codec::Atrac3, 102, Wireformat::L105kbps),
                "lp4" => (atracdenc::Codec::Atrac3Lp4, 64, Wireformat::Lp4),
                _ => unreachable!(),
            };
            let wav_file = std::fs::File::open(wav_tmp.path())
                .with_context(|| format!("reopening temp WAV {}", wav_tmp.path().display()))?;
            let encoded = atracdenc::EncodeBuilder::new()
                .codec(codec)
                .container(atracdenc::Container::Riff)
                .input_reader(std::io::BufReader::new(wav_file))
                .at3_settings(atracdenc::At3Settings {
                    bitrate_kbps: Some(bitrate),
                    ..Default::default()
                })
                .run_to_vec()
                .context("encoding ATRAC3 with atracdenc crate")?;
            // The temp WAV is no longer needed once encoding is done.
            drop(wav_tmp);

            let (detected, payload) = wav::atrac3_info(&encoded)
                .context("ATRAC encoder output was not a recognizable ATRAC3 WAV")?;
            if detected != wf {
                info!("note: ATRAC encoder produced {detected:?}, requested {wf:?}");
            }
            Ok(PreparedAudio {
                wireformat: detected,
                source: netmd::TrackSource::Memory(payload.to_vec()),
                _temp: None,
            })
        }
        other => bail!("unknown format {other:?} (expected sp, lp2, lp105, lp4)"),
    }
}
