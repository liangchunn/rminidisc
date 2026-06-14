//! NetMD device protocol library.
//!
//! Provides command functions for talking to a Sony NetMD device over USB.
//! The crate root is intentionally a small facade; implementation details live
//! in focused modules by protocol area.

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

pub use commands::{count_tracks_in_disc, get_device_status, list_content, tracks};
pub use descriptor::change_descriptor_state;
pub use device::{
    close_device, device_ids, list_connected_devices, open_device, open_device_matching,
    supported_device, DeviceDefinition, DeviceFlags, DeviceSelector, SHARP_VENDOR_ID,
    SONY_VENDOR_ID, SUPPORTED_DEVICES,
};
pub use disc::{
    get_disc_capacity, get_disc_flags, get_disc_subunit_identifier, get_disc_title, get_disk_title,
    rename_disc, set_disc_title,
};
pub use error::{NetMDError as Error, Result};
pub use groups::{
    cells_for_title, chars_to_cells, compile_disc_titles, get_track_group_list,
    remaining_characters_for_titles, rewrite_disc_groups, CompiledTitles, RawTrackGroup,
    RemainingChars, TitleCells,
};
pub use playback::{
    can_eject_disc, eject_disc, fast_forward, get_playback_status1, get_playback_status2,
    get_position, get_recording_parameters, goto_time, goto_track, next_track, pause, play,
    previous_track, restart_track, rewind, stop,
};
pub use secure::{
    acquire, commit_track, disable_new_track_protection, enter_secure_session, get_leaf_id,
    leave_secure_session, prepare_download, release, send_key_data, send_track,
    session_key_exchange, session_key_forget, setup_download, terminate,
};
pub use status::{get_full_operating_status, get_operating_status, get_status, is_disc_present};
pub use track_info::{
    erase_disc, erase_track, get_track_count, get_track_encoding, get_track_flags, get_track_info,
    get_track_length, get_track_title, move_track, set_track_title,
};
pub use transport::{
    read_reply, read_reply_checked, read_reply_length, send_query, send_query_ext, write_bulk,
    BULK_WRITE_ENDPOINT,
};
pub use types::{
    ChannelCount, Channels, DeviceStatus, Disc, DiscFlag, DiscFlags, DiscFormat, Encoding,
    FullOperatingStatus, Group, NetMDLevel, OperatingStatus, PlaybackState, PlaybackTime,
    ProtocolReply as Status, Track, TrackFlag, Wireformat, FRAME_SIZE,
};
pub use util::{format_time_from_frames, time_to_frames};
