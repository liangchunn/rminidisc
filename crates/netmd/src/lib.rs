pub mod commands;
pub mod crypto;
pub mod descriptor;
pub mod device;
pub mod disc;
pub mod ekb;
pub mod error;
pub mod groups;
pub mod playback;
pub mod query;
pub mod scan;
pub mod secure;
pub mod status;
pub mod title;
pub mod track;
pub mod track_info;
pub mod transport;
pub mod types;
pub mod util;
pub mod wav;

pub use device::NetMD;
pub use device::{
    list_connected_devices, open_device, open_device_matching, supported_device, DeviceDefinition,
    DeviceFlags, DeviceSelector, SHARP_VENDOR_ID, SONY_VENDOR_ID, SUPPORTED_DEVICES,
};
pub use error::{NetMDError as Error, Result};
pub use groups::{
    cells_for_title, chars_to_cells, compile_disc_titles, remaining_characters_for_titles,
    CompiledTitles, RawTrackGroup, RemainingChars, TitleCells,
};
pub use track::MdTrack;
pub use transport::BULK_WRITE_ENDPOINT;
pub use types::{
    ChannelCount, Channels, DeviceStatus, Disc, DiscFlag, DiscFlags, DiscFormat, Encoding,
    FullOperatingStatus, Group, OperatingStatus, PlaybackState, PlaybackTime,
    ProtocolReply as Status, Track, TrackFlag, Wireformat, FRAME_SIZE,
};
pub use util::{format_time_from_frames, time_to_frames};
