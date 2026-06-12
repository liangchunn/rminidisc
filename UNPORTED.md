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

## 3. Secure Upload / Download Pipeline (Phase 8/9 — deferred by decision)

The entire track-write (download-to-device) and track-read (upload-from-device)
pipeline. Reference: `netmd-interface.ts:709-912`, `MDTrack`/`MDSession`
(`netmd-interface.ts:944-1153`), `netmd-ekb.ts`, `encrypt-generator.ts`.

### 3a. USB Bulk Transfers (prerequisite)

Not yet implemented in Rust. Needed by `saveTrackToArray` (read) and `sendTrack`
(write).

- `readBulk(length, chunksize=0x10000, callback)` — `netmd.ts:211`
- `writeBulk(data)` — bulk OUT endpoint
- Endpoints: bulk IN `0x81`? / bulk OUT `0x02`? — confirm via descriptor.
  In `rusb` use `read_bulk` / `write_bulk` on the claimed interface endpoints.

### 3b. Crypto (prerequisite)

No DES crate is currently a dependency. Needs `des` + `cbc`/`ecb` (RustCrypto).

- `retailmac(key, value, iv)` — DES-CBC then TripleDES-CBC MAC
  (`netmd-interface.ts:915`). Used to derive the session key.
- DES-CBC packet encryption (per-frame) — `MDTrack.getPacketIterator`
  (`netmd-interface.ts:1036`).
- DES-ECB for `commitTrack` authentication (`netmd-interface.ts:826`).
- DES-CBC `NoPadding` for `setupDownload` content-id+kek encryption
  (`netmd-interface.ts:810`).

### 3c. EKB (Key Exchange Block)

`netmd-js/src/netmd-ekb.ts`:

- `EKBOpenSource` — hardcoded root key, EKB ID, key chain, signature.
- `CorruptedDeckEKB` — device-specific (MDS-JB980/JE780/NT1).
- `getEKBForDevice(leafID, vid, pid)` — selection logic.

### 3d. Secure Session Lifecycle (all prefix `1800 080046 f0030103`)

`netmd-interface.ts`:

- `getLeafID` (`11 ff`), `enterSecureSession` (`80 ff`),
  `leaveSecureSession` (`81 ff`)
- `sendKeyData(ekbid, keychain, depth, sig)` (`12 ff …`)
- `sessionKeyExchange(hostnonce)` (`20 ff 000000 …`)
- `sessionKeyForget` (`21 ff 000000`)
- `setupDownload`, `disableNewTrackProtection`, `saveTrackToArray`,
  `sendTrack`, `commitTrack`, `getTrackUUID`, `terminate`

### 3e. HiMD Mode

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
