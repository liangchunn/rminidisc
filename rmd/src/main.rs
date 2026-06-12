//! `rmd` — thin runner over the `netmd` library.
//!
//! Commands:
//!   rmd [--device VID:PID]
//!                     — dump disc + track metadata (default)
//!   rmd [--device VID:PID] info
//!                     — same as above
//!   rmd [--device VID:PID] upload <file> [--format sp|lp2|lp105|lp4] [--title T]
//!                     — encode (if needed) and write a track to the device
//!   rmd [--device VID:PID] erase [<track> | disc]
//!                     — erase a single track (1-based, as shown by `info`),
//!                       or the whole disc when given `disc` or no argument
//!   rmd [--device VID:PID] rename <track | disc> <title> [--full]
//!                     — set a track or disc title; `--full` writes the
//!                       full-width title field
//!   rmd [--device VID:PID] move <from> <to>
//!                     — reorder a track (1-based indexes)
//!
//! Device interaction is delegated to the `netmd` crate; this binary only does
//! argument parsing and host-side audio preparation (ffmpeg / atracdenc).

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context};
use log::info;
use netmd::track::MdTrack;
use netmd::wav;
use netmd::Wireformat;

fn main() -> anyhow::Result<()> {
    env_logger::init();

    let args = parse_global_args(std::env::args().skip(1))?;
    match args.get(1).map(String::as_str) {
        None | Some("info") => cmd_info(args.device),
        Some("upload") => cmd_upload(&args.rest[2..], args.device),
        Some("erase") => cmd_erase(&args.rest[2..], args.device),
        Some("rename") => cmd_rename(&args.rest[2..], args.device),
        Some("move") => cmd_move(&args.rest[2..], args.device),
        Some(other) => {
            bail!(
                "unknown command {other:?}. usage: rmd [--device VID:PID] [info | \
                 upload <file> [--format F] [--title T] | \
                 erase [<track> | disc] | \
                 rename <track | disc> <title> [--full] | \
                 move <from> <to>]"
            )
        }
    }
}

struct Args {
    device: Option<netmd::DeviceSelector>,
    rest: Vec<String>,
}

impl Args {
    fn get(&self, index: usize) -> Option<&String> {
        self.rest.get(index)
    }
}

fn parse_global_args<I>(args: I) -> anyhow::Result<Args>
where
    I: IntoIterator<Item = String>,
{
    let mut device = None;
    let mut rest = vec![String::from("rmd")];
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        if arg == "--device" || arg == "-d" {
            let value = args.next().context("--device requires VID:PID")?;
            let selector = parse_device_selector(&value)?;
            if device.replace(selector).is_some() {
                bail!("--device specified more than once");
            }
        } else if let Some(value) = arg.strip_prefix("--device=") {
            let selector = parse_device_selector(value)?;
            if device.replace(selector).is_some() {
                bail!("--device specified more than once");
            }
        } else {
            rest.push(arg);
        }
    }

    Ok(Args { device, rest })
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

    for track in 0..track_count as u16 {
        let title = netmd::get_track_title(&handle, track, false).unwrap_or_default();
        let length = netmd::get_track_length(&handle, track)?;
        let (encoding, channels) = netmd::get_track_encoding(&handle, track)?;
        let flags = netmd::get_track_flags(&handle, track)?;
        info!(
            "track {}: {:?} len={:02}:{:02}:{:02}+{:03} enc={:?} ch={:?} flags=0x{:02x}",
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

fn cmd_erase(args: &[String], device: Option<netmd::DeviceSelector>) -> anyhow::Result<()> {
    let target = args.first().map(String::as_str);
    if args.len() > 1 {
        bail!("usage: rmd erase [<track> | disc]");
    }

    let handle = netmd::open_device_matching(device)?;
    match target {
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

fn cmd_rename(args: &[String], device: Option<netmd::DeviceSelector>) -> anyhow::Result<()> {
    let mut target: Option<String> = None;
    let mut title: Option<String> = None;
    let mut full = false;

    for arg in args {
        match arg.as_str() {
            "--full" => full = true,
            other if target.is_none() => target = Some(other.to_string()),
            other if title.is_none() => title = Some(other.to_string()),
            other => bail!("unexpected argument {other:?}"),
        }
    }

    let target = target.context("usage: rmd rename <track | disc> <title> [--full]")?;
    let title = title.context("usage: rmd rename <track | disc> <title> [--full]")?;

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

fn cmd_move(args: &[String], device: Option<netmd::DeviceSelector>) -> anyhow::Result<()> {
    if args.len() != 2 {
        bail!("usage: rmd move <from> <to>");
    }
    let source = parse_track_index(&args[0])?;
    let dest = parse_track_index(&args[1])?;

    let handle = netmd::open_device_matching(device)?;
    netmd::move_track(&handle, source, dest)?;
    info!("moved track #{} -> #{}", source + 1, dest + 1);
    netmd::close_device(&handle)?;
    Ok(())
}

struct UploadArgs {
    file: String,
    format: String,
    title: Option<String>,
}

fn parse_upload_args(args: &[String]) -> anyhow::Result<UploadArgs> {
    let mut file: Option<String> = None;
    let mut format = "sp".to_string();
    let mut title: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--format" | "-f" => {
                i += 1;
                format = args.get(i).context("--format requires a value")?.clone();
            }
            "--title" | "-t" => {
                i += 1;
                title = Some(args.get(i).context("--title requires a value")?.clone());
            }
            other if !other.starts_with('-') => {
                if file.is_none() {
                    file = Some(other.to_string());
                } else {
                    bail!("unexpected argument {other:?}");
                }
            }
            other => bail!("unknown option {other:?}"),
        }
        i += 1;
    }

    Ok(UploadArgs {
        file: file.context("upload requires an input file")?,
        format,
        title,
    })
}

fn cmd_upload(args: &[String], device: Option<netmd::DeviceSelector>) -> anyhow::Result<()> {
    let args = parse_upload_args(args)?;
    let requested = args.format.to_lowercase();

    let default_title = Path::new(&args.file)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("track")
        .to_string();
    let title = args.title.unwrap_or(default_title);

    info!("preparing {:?} as {}", args.file, requested);
    let (wireformat, data) = prepare_audio(&args.file, &requested)?;
    info!(
        "prepared {} bytes, wire format {:?}",
        data.len(),
        wireformat
    );

    let track = MdTrack {
        title: title.clone(),
        full_width_title: None,
        format: wireformat,
        data,
    };

    let handle = netmd::open_device_matching(device)?;
    let (vendor, product) = netmd::device_ids(&handle)?;

    let mut last_pct = u64::MAX;
    let mut progress = |written: u64, total: u64| {
        let pct = written.saturating_mul(100).checked_div(total).unwrap_or(0);
        if pct != last_pct {
            info!("transferred {written} / {total} bytes ({pct}%)");
            last_pct = pct;
        }
    };

    let (track_num, uuid, ccid) =
        netmd::track::download_track(&handle, &track, vendor, product, Some(&mut progress))?;

    info!("uploaded track #{track_num} (uuid={uuid} ccid={ccid}) title={title:?}");

    netmd::close_device(&handle)?;
    Ok(())
}

/// Prepares audio for upload, returning the wire format and raw payload.
///
/// - If the input is an ATRAC3 WAV, its payload is used directly (header
///   stripped) and the requested format is ignored (the file dictates it).
/// - SP: transcode to big-endian s16 PCM via ffmpeg.
/// - LP2/LP105/LP4: transcode to a 44.1 kHz stereo WAV via ffmpeg, encode with
///   atracdenc, then strip the resulting ATRAC3 WAV header.
fn prepare_audio(input: &str, requested: &str) -> anyhow::Result<(Wireformat, Vec<u8>)> {
    let raw = std::fs::read(input).with_context(|| format!("reading {input}"))?;

    // Already ATRAC3? Use it directly.
    if let Some((fmt, payload)) = wav::atrac3_info(&raw) {
        info!("input is ATRAC3 ({fmt:?}); using payload directly");
        return Ok((fmt, payload.to_vec()));
    }

    match requested {
        "sp" => {
            // ffmpeg -i in -ac 2 -ar 44100 -f s16be out.raw
            let out = temp_path("raw");
            run_ffmpeg(&[
                "-y", "-i", input, "-ac", "2", "-ar", "44100", "-f", "s16be", &out,
            ])?;
            let data = std::fs::read(&out)?;
            let _ = std::fs::remove_file(&out);
            Ok((Wireformat::Pcm, data))
        }
        "lp2" | "lp105" | "lp4" => {
            // First make a clean 44.1k stereo WAV via ffmpeg.
            let wav_in = temp_path("wav");
            run_ffmpeg(&[
                "-y", "-i", input, "-ar", "44100", "-ac", "2", "-f", "wav", &wav_in,
            ])?;

            // Encode to ATRAC3 (RIFF container) via atracdenc.
            let (codec, bitrate, wf) = match requested {
                "lp2" => ("atrac3", "128", Wireformat::Lp2),
                "lp105" => ("atrac3", "102", Wireformat::L105kbps),
                "lp4" => ("atrac3_lp", "64", Wireformat::Lp4),
                _ => unreachable!(),
            };
            let atrac_out = temp_path("at3.wav");
            run_atracdenc(&[
                "-e",
                codec,
                "-i",
                &wav_in,
                "-o",
                &atrac_out,
                "--container",
                "riff",
                "--bitrate",
                bitrate,
            ])?;

            let encoded = std::fs::read(&atrac_out)?;
            let _ = std::fs::remove_file(&wav_in);
            let _ = std::fs::remove_file(&atrac_out);

            let (detected, payload) = wav::atrac3_info(&encoded)
                .context("atracdenc output was not a recognizable ATRAC3 WAV")?;
            if detected != wf {
                info!("note: atracdenc produced {detected:?}, requested {wf:?}");
            }
            Ok((detected, payload.to_vec()))
        }
        other => bail!("unknown format {other:?} (expected sp, lp2, lp105, lp4)"),
    }
}

fn temp_path(ext: &str) -> String {
    let dir = std::env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    dir.join(format!("rmd_{nanos}.{ext}"))
        .to_string_lossy()
        .into_owned()
}

fn run_ffmpeg(args: &[&str]) -> anyhow::Result<()> {
    let status = Command::new("ffmpeg")
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("failed to run ffmpeg (is it installed?)")?;
    if !status.success() {
        bail!("ffmpeg failed with status {status}");
    }
    Ok(())
}

fn run_atracdenc(args: &[&str]) -> anyhow::Result<()> {
    // Prefer an ATRACDENC env override, else the in-tree arm64 build.
    let bin = std::env::var("ATRACDENC")
        .unwrap_or_else(|_| "atracdenc/build-arm64/src/atracdenc".to_string());
    let status = Command::new(&bin)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .with_context(|| format!("failed to run atracdenc at {bin}"))?;
    if !status.success() {
        bail!("atracdenc failed with status {status}");
    }
    Ok(())
}
