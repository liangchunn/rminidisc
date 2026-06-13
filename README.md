# rminidisc

Rust workspace for interacting with Sony/Sharp NetMD MiniDisc devices over USB.

## Crates

### `netmd` — Library

Low-level protocol library for NetMD devices. Provides:

- Device enumeration, open/close (`open_device_matching`, `list_connected_devices`)
- Disc + track metadata (title, flags, encoding, length)
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
| `info` (default) | Dump disc + track metadata |
| `upload -f <fmt> <file>` | Encode and write a track (sp, lp2, lp105, lp4) |
| `upload --folder <dir>` | Upload all files in a directory |
| `erase [track\|disc]` | Erase a track or the whole disc |
| `rename <track\|disc> <title>` | Set a track or disc title |
| `move <from> <to>` | Reorder a track |
| `play` / `pause` / `stop` | Playback transport |
| `next` / `prev` / `ff` / `rewind` | Track navigation |
| `eject` | Eject the disc |
| `goto <track>` | Seek to track start |
| `status` | One-shot playback status |
| `list` | List connected NetMD devices |
| `control` | Interactive playback TUI |

Select a device with `-d VID:PID` (e.g. `-d 054c:0084`).

## Upload Dependencies

Uploading non-ATRAC3 audio requires external binaries:

- **ffmpeg** — PCM/SP conversion and WAV preparation
- **[atracdenc](https://github.com/dcherednik/atracdenc)** — LP2/LP4 ATRAC3 encoding

Override their paths via environment variables:

| Variable | Default | Purpose |
|----------|---------|---------|
| `FFMPEG` | `ffmpeg` | Path to ffmpeg binary |
| `ATRACDENC` | `atracdenc` | Path to atracdenc binary |

ATRAC3 `.wav` files are uploaded directly without transcoding.
