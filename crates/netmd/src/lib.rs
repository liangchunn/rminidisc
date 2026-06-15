//! Talk to Sony/Sharp NetMD MiniDisc recorders over USB.
//!
//! `netmd` is a Rust port of the [`netmd-js`](https://github.com/cybercase/netmd-js)
//! TypeScript library. It speaks the NetMD control protocol over `rusb`/libusb:
//! enumerating supported devices, reading and editing disc/track titles and
//! groups, controlling playback, and uploading audio through the secure
//! download pipeline.
//!
//! # Quick start
//!
//! The high-level surface hangs off the [`NetMD`] handle. Open a device, then
//! call methods on it. Most reads return owned data; all fallible calls return
//! [`Result`].
//!
//! ```no_run
//! use netmd::{open_device, Result};
//!
//! fn main() -> Result<()> {
//!     // Auto-select the single connected supported device.
//!     let netmd = open_device()?;
//!
//!     // Enumerate the whole disc in one structured call.
//!     let disc = netmd.list_content()?;
//!     println!("disc: {:?} ({} tracks)", disc.title, disc.track_count);
//!     for group in &disc.groups {
//!         for track in &group.tracks {
//!             println!(
//!                 "  track {}: {:?} ({} frames)",
//!                 track.index + 1,
//!                 track.title.as_deref().unwrap_or(""),
//!                 track.duration_frames,
//!             );
//!         }
//!     }
//!
//!     netmd.close()?;
//!     Ok(())
//! }
//! ```
//!
//! # Selecting a device
//!
//! With multiple supported recorders attached, pass a [`DeviceSelector`] built
//! from a USB vendor/product ID pair. Use [`list_connected_devices`] to
//! discover what is present and [`supported_device`] to validate an ID pair.
//!
//! ```no_run
//! use netmd::{open_device_matching, DeviceSelector, Result};
//!
//! fn main() -> Result<()> {
//!     for dev in netmd::list_connected_devices()? {
//!         println!("found {} ({:04x}:{:04x})", dev.name, dev.vendor_id, dev.product_id);
//!     }
//!
//!     // Sony MZ-N1, for example.
//!     let selector = DeviceSelector::new(0x054c, 0x0081);
//!     let netmd = open_device_matching(Some(selector))?;
//!     netmd.close()?;
//!     Ok(())
//! }
//! ```
//!
//! # Editing titles and groups
//!
//! Track indices are 0-based throughout the API. The `w_char` flag selects the
//! full-width (wide-character) title table versus the half-width table.
//!
//! ```no_run
//! use netmd::{open_device, Result};
//!
//! fn main() -> Result<()> {
//!     let netmd = open_device()?;
//!     netmd.set_disc_title("My Mixtape", false)?;
//!     netmd.set_track_title(0, "Opening Track", false)?;
//!     netmd.move_track(2, 0)?; // move 3rd track to the front
//!     netmd.close()?;
//!     Ok(())
//! }
//! ```
//!
//! # Playback control
//!
//! ```no_run
//! use netmd::{open_device, Result};
//!
//! fn main() -> Result<()> {
//!     let netmd = open_device()?;
//!     netmd.goto_track(0)?;
//!     netmd.play()?;
//!     // ...
//!     netmd.stop()?;
//!     netmd.close()?;
//!     Ok(())
//! }
//! ```
//!
//! # Uploading audio
//!
//! Uploads go through the encrypted secure-download pipeline. The payload must
//! already be in a NetMD wire format: big-endian s16 PCM for [`Wireformat::Pcm`]
//! (SP), or raw ATRAC3 frames for the LP formats. The `md-pcm` crate in this
//! workspace handles decoding/resampling/encoding to produce that payload.
//!
//! ```no_run
//! use netmd::{open_device, track::MdTrack, Result, Wireformat};
//!
//! fn main() -> Result<()> {
//!     let netmd = open_device()?;
//!     let track = MdTrack {
//!         title: "New Track".to_string(),
//!         full_width_title: None,
//!         format: Wireformat::Pcm,
//!         data: std::fs::read("track.s16be.raw").unwrap(),
//!     };
//!
//!     let mut progress = |written: u64, total: u64| {
//!         println!("{written}/{total} bytes");
//!     };
//!     let (track_num, uuid, ccid) = netmd.download_track(&track, Some(&mut progress))?;
//!     println!("uploaded track #{track_num} (uuid={uuid} ccid={ccid})");
//!     netmd.close()?;
//!     Ok(())
//! }
//! ```
//!
//! # Module map
//!
//! The public surface is intentionally small. [`NetMD`] (defined in [`device`])
//! is the main handle; its methods cover all device operations. Everything below
//! the handle — query/reply framing, the secure-session command set, crypto,
//! descriptors, and parsing primitives — is crate-private implementation detail.
//!
//! Public modules:
//!
//! - [`device`] — device enumeration, opening/claiming, and the [`NetMD`] handle.
//!   The inherent methods on [`NetMD`] provide disc-level edits, per-track
//!   info/erase/move, playback transport control, operating-status polling, and
//!   the high-level aggregates [`NetMD::list_content`] / [`NetMD::get_device_status`].
//! - [`groups`] — track-group listing and editing helpers.
//! - [`track`] — [`MdTrack`] and [`NetMD::download_track`], the upload entry point.
//! - [`types`] — shared data types ([`Disc`], [`Track`], [`Group`], [`Encoding`], ...).
//! - [`wav`] — WAV/ATRAC3 payload inspection helpers used when preparing uploads.
//! - [`error`] — [`Error`]/[`Result`].
//!
//! Convenience re-exports for the most common types are available at the crate
//! root (see the re-export list below).

pub(crate) mod commands;
pub(crate) mod crypto;
pub(crate) mod descriptor;
pub mod device;
pub(crate) mod disc;
pub(crate) mod ekb;
pub mod error;
pub mod groups;
pub(crate) mod playback;
pub(crate) mod query;
pub(crate) mod scan;
pub(crate) mod secure;
pub(crate) mod status;
pub(crate) mod title;
pub mod track;
pub(crate) mod track_info;
pub(crate) mod transport;
pub mod types;
pub(crate) mod util;
pub mod wav;

pub use device::NetMD;
pub use device::{
    list_connected_devices, open_device, open_device_matching, supported_device, DeviceDefinition,
    DeviceFlags, DeviceSelector, SHARP_VENDOR_ID, SONY_VENDOR_ID, SUPPORTED_DEVICES,
};
pub use error::{NetMDError as Error, Result};
pub use groups::{chars_to_cells, CompiledTitles, RawTrackGroup, RemainingChars, TitleCells};
pub use track::MdTrack;
pub use types::{
    ChannelCount, Channels, DeviceStatus, Disc, DiscFlag, DiscFlags, DiscFormat, Encoding,
    FullOperatingStatus, Group, OperatingStatus, PlaybackState, PlaybackTime, Track, TrackFlag,
    Wireformat,
};
pub use util::{format_time_from_frames, time_to_frames};
