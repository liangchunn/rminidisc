# NetMD Core Functionality — TODO

Minimal set of features needed for basic NetMD device interaction.

---

## 1. Infrastructure & Device Setup

### USB Layer
- [x] Device open, claim interface (main.rs)
- [x] `send_query()` — USB control transfer send (request 0x80)
- [x] `read_reply()` — USB control transfer read (0x01 header + 0x81 data); now polls length until non-zero
- [x] Reply status checking — `0x08`→NotImplemented, `0x0a`→Rejected, `0x0f`→Interim retry w/ backoff (`error.rs`, `read_reply_checked`)
- [x] `acquire()` / `release()` — device lock (`ff 010c` / `ff 0100`)

### Discovery
- [x] `_getDiscSubunitIdentifier()` — `1809 00 ff00 0000 0000` (returns NetMD level; verified 0x20 on MZ-N505)

### Status
- [x] `getStatus()` — `1809 8001 0230 8800 0030 8804 00 ff00 00000000`
- [x] `getOperatingStatus()` / `getFullOperatingStatus()`
- [x] `isDiscPresent()` (bonus, verified)

---

## 2. Descriptor State Management
- [x] `changeDescriptorState()` — open/close TDs

---

## 3. Disc Title Read/Write

### Read
- [x] `get_disk_title(handle, wchar)` — raw hex query + scan (now self-manages descriptors)
- [x] `getDiscTitle(wchar)` — wrapper with group-delimiter trimming + open/close (verified "MD1")

### Write
- [x] `setDiscTitle(title, wchar)` — `1807 02201801 00{wc} 3000 0a00 5000 {newlen} 0000 {oldlen} {sjis_bytes}` (code-complete; sanitization deferred → UNPORTED.md; not run on disc)

### Supporting Utilities
- [x] `parse_string(sjis_bytes)` — SHIFT_JIS→UTF-8
- [x] `encode_to_sjis(utf8)` — UTF-8→SHIFT_JIS
- [x] `get_length_after_sjis_encode(utf8)` — byte length after encoding

---

## 4. Track Title Read/Write

### Read
- [x] `getTrackTitle(track, wchar)` — `1806 022018{wc} {track} 3000 0a00 ff00 00000000` (verified all 18 tracks)

### Write
- [x] `setTrackTitle(track, title, wchar)` — `1807 022018{wc} {track} 3000 0a00 5000 {new} 0000 {old} {sjis_bytes}` (code-complete; sanitization deferred; not run on disc)

---

## 5. Track Information

### Track Count
- [x] `getTrackCount()` — `1806 02101001 3000 1000 ff00 00000000`

### Per-Track Info
- [x] `getTrackFlags(track)` — `1806 01201001 {track} ff00 00010008` (verified)
- [x] `_getTrackInfo(track, p1, p2)` — `1806 02201001 {track} {p1} {p2} ff00 00000000`
- [x] `getTrackEncoding(track)` — parses `8007 0004 0110 %b %b` from rawValue (verified 0x90 SP)
- [x] `getTrackLength(track)` — parses `0001 0006 0000 %B %B %B %B` from rawValue (verified)

### Disc Info
- [x] `getDiscFlags()` — `1806 01101000 ff00 0001000b` (verified 0x10 writable)
- [x] `getDiscCapacity()` — `1806 02101000 3080 0300 ff00 00000000` (verified)

---

## 6. Track Deletion
- [x] `eraseTrack(track)` — `1840 ff01 00 201001 {track}` (code-complete; NOT run on disc — destructive)

---

## 7. Track Reordering
- [x] `moveTrack(source, dest)` — `1843 ff00 00 201001 {src} 201001 {dst}` (code-complete; NOT run on disc — destructive)

---

## 8. Disc Wipe
- [x] `eraseDisc()` — `1840 ff 0000` (code-complete; NOT run on disc — destructive)

---

## 9. Secure Upload (Writing Tracks to Device)

> **DEFERRED** (except the enums below). The EKB, secure session lifecycle,
> crypto (DES retailmac / packet encryption), USB bulk transfers, and the track
> upload pipeline are **not ported**. They require new dependencies (DES crates)
> and hardware that can't be safely verified here. See `UNPORTED.md` §3.

### Security Enums
- [x] `DiscFormat` — `lp4(0), lp2(2), spMono(4), spStereo(6)` (types.rs)
- [x] `Wireformat` — `pcm(0), l105kbps(0x90), lp2(0x94), lp4(0xa8)` (types.rs)
- [x] `Encoding` — `sp(0x90), lp2(0x92), lp4(0x93)` (types.rs)
- [x] `TrackFlag` — `protected(0x03), unprotected(0x00)` (types.rs)
- [x] `FrameSize` — pcm:2048, lp2:192, l105kbps:152, lp4:96 (`FRAME_SIZE`)

### EKB (Key Exchange Block)
- [ ] `EKBOpenSource` — hardcoded root key, EKB ID, key chain, signature
- [ ] `getEKBForDevice(leafID, vid, pid)` — EKB selection

### Secure Session Lifecycle
All use prefix `1800 080046 f0030103`.
- [ ] `getLeafID()` — `11 ff` → `11 00 %*`
- [ ] `enterSecureSession()` — `80 ff` → `80 00`
- [ ] `leaveSecureSession()` — `81 ff` → `81 00`
- [ ] `sendKeyData(ekbid, keychain, depth, sig)` — `12 ff {ekbid} 0000 {keylen} {keys...} {depth} 00000000 {sig}`
- [ ] `sessionKeyExchange(hostnonce)` — `20 ff 000000 {nonce}` → `20 %? 000000 {devnonce}`
- [ ] `sessionKeyForget()` — `21 ff 000000` → `21 00 000000`

### Crypto
- [ ] `retailmac(key, value, iv?)` — DES-CBC-MAC for session key derivation
- [ ] DES-CBC packet encryption — chunk data into frames, encrypt with session key

### Track Upload Pipeline
- [ ] `setupDownload(contentid, kek, hexSessionKey)` — `22 ff 0000 {encrypted_contentid_kek}`
- [ ] `disableNewTrackProtection(track)` — `2b ff {track}` → `2b 00 %?%?`
- [ ] `saveTrackToArray(track, callback?)` — reads track metadata into device memory
- [ ] `sendTrack(wireformat, discformat, frames, pktSize, packets, hexSessionKey, cb?)` — `28 ff 000100 1001 ffff 00 {wc} {df} {frames} {pktdata...}`
- [ ] `commitTrack(tracknum, hexSessionKey)` — `48 ff 00 1001 {track} {encrypted_sessionkey}`
- [ ] `terminate()` — `2a ff00` — ends upload process

---

## 10. Supporting Utilities

### String Encoding
- [x] `parse_string(sjis_bytes)` — SHIFT_JIS→UTF-8
- [x] `encode_to_sjis(utf8_string)` — UTF-8→SHIFT_JIS (util.rs)
- [x] `get_length_after_sjis_encode(utf8)` — SJIS byte length

### Scan Directives
- [x] `%?` `%b` `%w` `%d` `%q` `%*`
- [x] `%B` `%W` — BCD-encoded values (raw slice; decode via `parse_bcd_u8`/`parse_bcd_u16`)
- [x] `%x` `%s` `%z` — length-prefixed data
- [x] `%#` — all-remaining (consume-to-end, matches JS); `%<`/`%>` markers accepted/skipped (see UNPORTED.md)

### Time Formatting
- [x] `format_time_from_frames(frames)` — frames→`HH:MM:SS+FFF`
- [x] `time_to_frames([h,m,s,f])` — time array→absolute frames

### Data Types
- [x] `ReadRequestHeader` / `ReadRequestData`
- [x] `Query` / `ProtocolReply`
- [x] `DiscFormat` / `Wireformat` / `Encoding` / `TrackFlag` / `DiscFlag` / `FrameSize` (`FRAME_SIZE`) / `Channels` / `ChannelCount` / `NetMDLevel`

---

## Per-Feature Dependency Summary

| Feature | Dependencies | Status |
|---|---|---|
| Read disc title | wrapper + open/close | ✅ done, verified |
| Write disc title | `encode_to_sjis`, open/close flow | ✅ code-complete (not run on disc) |
| Read track titles | `getTrackTitle` hex, SJIS decode | ✅ done, verified |
| Read track info (length, encoding, flags) | `getTrackInfo` + BCD scan | ✅ done, verified |
| Delete track | `eraseTrack` hex only | ✅ code-complete (not run on disc) |
| Reorder track | `moveTrack` hex only | ✅ code-complete (not run on disc) |
| Wipe disc | `eraseDisc` hex only | ✅ code-complete (not run on disc) |
| Upload track | EKB → secure session → sendTrack + DES-CBC | ⛔ deferred → UNPORTED.md |
