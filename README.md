# rminidisc

Rust workspace for interacting with Sony/Sharp NetMD MiniDisc devices over USB.

## Crates

### `netmd` — Library

Low-level protocol library for NetMD devices. Provides:

- Device enumeration, open/close (`open_device_matching`, `list_connected_devices`)
- Disc + track metadata (title, flags, encoding, length)
- Structured disc listing with track groups (`list_content`, `get_track_group_list`)
- Group editing + TOC title-cell budgeting (`rewrite_disc_groups`, `remaining_characters_for_titles`)
- Track upload via secure session (ATRAC3 SP / LP2 / LP4)
- Erase, rename, reorder tracks
- Playback transport (play, pause, stop, seek, ff/rewind, eject)
- Supported devices: Sony (054c:*) and Sharp (04dd:*) NetMD recorders

### `rmd` — CLI Binary

```sh
cargo build --release -p rmd
```

Front-end for the `netmd` library. Subcommands:

| Command | Description |
|---------|-------------|
| `info` (default) | Dump disc + track metadata (grouped) |
| `upload -f <fmt> <file>` | Encode and write a track (sp, lp2, lp105, lp4) |
| `upload --folder <dir>` | Upload all files in a directory |
| `erase [track\|disc]` | Erase a track or the whole disc |
| `rename <track\|disc> <title>` | Set a track or disc title |
| `move <from> <to>` | Reorder a track |
| `groups` | List the disc's group structure + per-track details |
| `group add <range> <name>` | Create a group over a 1-based track range (`1-3` or `5`) |
| `group rename <index> <name>` | Rename a group (index from `groups`) |
| `group remove <index>` | Dissolve a group (index from `groups`) |
| `play` / `pause` / `stop` | Playback transport |
| `next` / `prev` / `ff` / `rewind` | Track navigation |
| `eject` | Eject the disc |
| `goto <track>` | Seek to track start |
| `status` | One-shot playback status |
| `list` | List connected NetMD devices |
| `control` | Interactive playback TUI |

Select a device with `-d VID:PID` (e.g. `-d 054c:0084`).

## Upload Audio

Uploading non-ATRAC3 audio is normalized in Rust with `symphonia` and `rubato`:

- SP uploads are decoded to stereo 44.1 kHz signed 16-bit big-endian PCM.
- LP2/LP105/LP4 uploads are decoded to stereo 44.1 kHz PCM WAV, then encoded with the local `atracdenc` crate.

Supported source formats are the formats supported by `symphonia`'s enabled decoders. ATRAC3 `.wav` files are uploaded directly without transcoding.
