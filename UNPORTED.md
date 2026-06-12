# Unported / To-Be-Researched

Items from `netmd-js` that were intentionally **not** ported during the
CORE_TODO Phases 1–7 pass. Each entry notes why it was deferred and where the
reference implementation lives.

---

## 1. Title Sanitization (ported)

`setDiscTitle` / `setTrackTitle` in the JS reference call:

- `sanitizeHalfWidthTitle(title)` — `netmd-js/src/utils.ts:244`
- `sanitizeFullWidthTitle(title)` — `netmd-js/src/utils.ts:317`

These involve large character-remapping tables:

- half-width ↔ full-width kana with (han)dakuten "flattening"
- Japanese / Russian / German special-character mappings
- `getHalfWidthTitleLength` multi-byte awareness (`utils.ts:230`)
- `halfWidthToFullWidthRange` (ported in `util.rs`? No — only used by groups)

**Rust behavior:** `set_disc_title` / `set_track_title` sanitize titles before
SHIFT_JIS encoding via `netmd/src/title.rs`, including half-width/full-width kana,
dakuten/handakuten flattening, Japanese / Russian / German mappings, and the
half-width length helper.

---

## 2. Sharp Vendor Quirk in `setDiscTitle` (ported)

`netmd-interface.ts:589-605`: for vendor `0x04dd` (Sharp), disc rename uses the
`audioUTOC1TD` descriptor with `openWrite`/`close` instead of the `discTitleTD`
open/close dance (webminidisc issue #67).

**Rust behavior:** `set_disc_title` detects vendor `0x04dd` and uses the
`audioUTOC1TD` open-write/close descriptor flow. This branch is covered by unit
tests but has not been exercised against real Sharp hardware in this repository.

---

## 3. Secure Upload / Download Pipeline

### Download-to-device (track-write) — PORTED ✅

The track-write (download-to-device) pipeline is now implemented and verified on
the MZ-N505 for SP, LP2, and LP4. Reference: `netmd-interface.ts:709-912`,
`MDTrack`/`MDSession` (`netmd-interface.ts:944-1153`), `netmd-ekb.ts`,
`encrypt-generator.ts`.

Rust locations:

- Crypto (`retailmac`, DES-CBC/ECB, TripleDES-CBC, packet encryptor) — `netmd/src/crypto.rs`
- EKB (`EKBOpenSource` only) — `netmd/src/ekb.rs`
- Query builder (`formatQuery` equivalent) — `netmd/src/query.rs` (`QueryBuilder`)
- Secure session commands + `write_bulk` + `send_track` + `prepare_download` — `netmd/src/lib.rs`
- `MdTrack` / `download_track` orchestration — `netmd/src/track.rs`
- WAV/ATRAC3 detection + data prep — `netmd/src/wav.rs`, `rmd/src/main.rs`
  (`rmd upload <file> [--format sp|lp2|lp105|lp4] [--title T]`)

Notable protocol details discovered during porting:

- Secure commands need the leading `00` status/pad byte (added by
  `NetMDInterface.sendCommand`, `netmd.ts:226`); `SECURE_PREFIX` includes it.
- The first bulk packet's `pktSize` header is **big-endian** (`sendTrack`
  reverses the LE buffer on LE hosts, `netmd-interface.ts:871`).
- Every reply read must be followed by a trailing `getReplyLength` poll
  (`netmd.ts:206`). This is required for the device's flow control after the
  bulk transfer — without it the device never produces the final `sendTrack`
  reply. Implemented in `read_reply` + `read_reply_after_bulk`.

### Track-read (upload-from-device) — STILL DEFERRED ❌

`saveTrackToArray` / `readBulk` (device → host) is **not** ported. It is
hardware-restricted to the MZ-RH1 / MZ-M200 (`netmd-interface.ts:710`: "This can
only be executed on an MZ-RH1 / M200") and cannot run on the MZ-N505. If a
supported device is added later:

- `readBulk(length, chunksize=0x10000, callback)` — `netmd.ts:211`, bulk IN `0x81`.
- `saveTrackToArray` — `netmd-interface.ts:709`.

### HiMD Mode — DEFERRED (out of scope)

- `enterHiMDMode` (`1800 080046 f0030104 82 ff`) — `netmd-interface.ts:741`
- `getLeafID` HiMD variant

---

## 4. Track Groups

`getTrackGroupList()` (`netmd-interface.ts:509`) parses the disc title's group
structure (`//`-delimited cells like `1-3;GroupName`). Read-only and would be
useful, but not in the CORE_TODO list. The raw title is already readable via
`get_disc_title`. Porting requires the half/full-width range helpers.

---

## 5. Playback Control (not in CORE_TODO)

`netmd-interface.ts:347-424`: `play`/`pause`/`stop`/`fast_forward`/`rewind`,
`gotoTrack`/`gotoTime`, `nextTrack`/`previousTrack`/`restartTrack`,
`ejectDisc`/`canEjectDisc`, `getPosition`, `getPlaybackStatus1/2`,
`getRecordingParameters`. Straightforward to port later (no crypto/bulk needed).

---

## 6. Factory Command Set (not in CORE_TODO)

`netmd-js/src/factory/`. Uses request code `0xff` instead of `0x80` (not yet
wired in the Rust USB layer). Memory read/write, display override, device
version/code, UTOC sector read/write, EEPROM checksum, etc. See `TODO.md`.

---

## 7. Worker-Thread Encryption (N/A for native)

`node-encrypt-worker.ts` / `web-encrypt-worker.ts` offload DES encryption to a
worker thread for the browser/Node event loop. Not needed for native Rust —
encryption can run inline or on a `std::thread`.

---

## Notes on Scan Directives

- `%<` / `%>` endianness markers are *accepted and skipped* by `scan.rs`; since
  `scan` returns raw byte slices, the caller chooses interpretation. No
  CORE_TODO feature needs little-endian scanning. If a future command does,
  decoding must respect the marker.
- `%#` is implemented identically to `%*` (consume-to-end), matching the JS
  `splice(0)` behavior, despite PORTING_REFERENCE.md calling it "non-consuming".
