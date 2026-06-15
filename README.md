# rminidisc

Pure Rust crates interfacing with NetMD MiniDisc via USB, including audio normalization and ATRAC3 encoding via [`atracdenc-rs`](https://github.com/liangchunn/atracdenc-rs).

No `ffmpeg` or `atracdenc` install required.

> [!WARNING]
> Disclaimer:
>
> The code in this repository are largely written and reviewed with the aid of AI LLMs, and verifying it with a real Sony MZ-N505 on macOS.
>
> I am not responsible for bricking your device! Use at your own risk!

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

**Original JS reference:** https://github.com/cybercase/netmd-js
**Ported from commit**: [`5919d93`](https://github.com/cybercase/netmd-js/tree/5919d93c3ae4375806c2d4248495a31f81830b82)

### `md-pcm` — Audio Decode/Normalize Library

Decodes source audio and normalizes it to stereo 44.1 kHz PCM for use by the
upload path. Replaces need for calling out to `ffmpeg`. Provides:

- Streaming decode + resample via `symphonia` and `rubato`
- Output as interleaved signed 16-bit big-endian PCM (`decode_to_s16be_44100_stereo*`)
- Output as 16-bit PCM WAV (`decode_to_wav_44100_stereo*`)
- Bounded memory: writer-based APIs (`*_writer`) stream incrementally

### `rminidisc` — CLI Binary

```sh
cargo build --release -p rminidisc
```

Front-end for the `netmd` library. Subcommands:

| Command                           | Description                                              |
| --------------------------------- | -------------------------------------------------------- |
| `info` (default)                  | Dump disc + track metadata (grouped)                     |
| `upload -f <fmt> <file>`          | Encode and write a track (sp, lp2, lp105, lp4)           |
| `upload --folder <dir>`           | Upload all files in a directory                          |
| `erase [track\|disc]`             | Erase a track or the whole disc                          |
| `rename <track\|disc> <title>`    | Set a track or disc title                                |
| `move <from> <to>`                | Reorder a track                                          |
| `groups`                          | List the disc's group structure + per-track details      |
| `group add <range> <name>`        | Create a group over a 1-based track range (`1-3` or `5`) |
| `group rename <index> <name>`     | Rename a group (index from `groups`)                     |
| `group remove <index>`            | Dissolve a group (index from `groups`)                   |
| `play` / `pause` / `stop`         | Playback transport                                       |
| `next` / `prev` / `ff` / `rewind` | Track navigation                                         |
| `eject`                           | Eject the disc                                           |
| `goto <track>`                    | Seek to track start                                      |
| `status`                          | One-shot playback status                                 |
| `list`                            | List connected NetMD devices                             |
| `control`                         | Interactive playback TUI                                 |

Select a device with `-d VID:PID` (e.g. `-d 054c:0084`).

## Why this project exists

This started when I was [looking into a better way to parse NetMD messages with a Rust macro](https://liangchun.me/posts/netmd-rust-macros) for funsies.

I intended to "finish" the project but never got the time to do so, therefore the AI port. I didn't include the scan macro into this project because most protocl message shapes are fixed anyway.

One thing after the other, I ended up with a functioning port, and decided to [port atracdenc from C++](https://github.com/liangchunn/atracdenc-rs) as well for the complete RIIR package.

## Acknowledgments

This project has been made possible by:

- [netmd-js](https://github.com/cybercase/netmd-js) (reference impl)
- [linux-minidisc](https://github.com/linux-minidisc/linux-minidisc)
- [hound](https://github.com/ruuda/hound): WAV decoding library
- [rubato](https://github.com/HEnquist/rubato): sample rate conversion
- [symphonia](https://github.com/pdeljanov/Symphonia): audio demux + decode
