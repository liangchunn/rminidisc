# NetMD Rust Port — TODO

Items marked ~~strikethrough~~ are already implemented in Rust.

---

## Enums & Constants

### `src/descriptor.rs`
- [x] `Descriptor` enum
- [x] `DescriptorAction` enum

### `src/types.rs`
- [x] `ProtocolReply` (Status codes)
- [x] `USB_TIMEOUT_MILLIS`

### NetMD Protocol Enums (netmd-interface.ts)
- [ ] `DiscFormat` — `lp4(0), lp2(2), spMono(4), spStereo(6)`
- [ ] `Wireformat` — `pcm(0), l105kbps(0x90), lp2(0x94), lp4(0xa8)`
- [ ] `Encoding` — `sp(0x90), lp2(0x92), lp4(0x93)`
- [ ] `Channels` — `mono(0x01), stereo(0x00)`
- [ ] `ChannelCount` — `mono(1), stereo(2)`
- [ ] `TrackFlag` — `protected(0x03), unprotected(0x00)`
- [ ] `DiscFlag` — `writable(0x10), writeProtected(0x40)`
- [ ] `NetMDLevel` — `level1(0x20), level2(0x50), level3(0x70)`

### Internal Enums (netmd-interface.ts:27–37)
- [ ] `Action` — `play(0x75), pause(0x7d), fastForward(0x39), rewind(0x49)`
- [ ] `Track` — `previous(0x0002), next(0x8001), restart(0x0001)`

### Constants
- [ ] `FrameSize` dict — pcm:2048, lp2:192, l105kbps:152, lp4:96

---

## Core Infrastructure

### Query/Reply Layer
- [x] `Query` struct — hex string → `Vec<u8>`
- [x] `scan()` — binary reply parser with `%b %w %d %q %? %*` directives
- [x] `send_query()` — USB control transfer send
- [x] `read_reply()` — USB control transfer read with header length

### Scan Directives Still Missing
- [ ] `%B` / `%W` — BCD-encoded byte/word
- [ ] `%x` / `%s` / `%z` — length-prefixed data (2-byte / null-terminated / 1-byte)
- [ ] `%#` — all remaining data (non-consuming)
- [ ] `%<` / `%>` — endianness override

### USB Layer
- [ ] `NetMD` class — device init, open, claim interface, finalize
- [ ] Factory command request code (`0xff`) — currently only normal `0x80`
- [ ] Factory reply request code (`0xff`) — currently only normal `0x81`
- [ ] Bulk read/write endpoints (`0x01`/`0x02`)
- [ ] Device list/detection (DevicesIds table, vendorId/productId matching)

---

## NetMDInterface Methods (netmd-interface.ts)

### Descriptor State Management
- [x] `changeDescriptorState()` — open/close descriptors
- [ ] `_getDiscSubunitIdentifier()` — query `1809 00 ff00 0000 0000`

### Device Locking
- [ ] `acquire()` — `ff 010c ffff ffff ffff ffff ffff ffff`
- [ ] `release()` — `ff 0100 ffff ffff ffff ffff ffff ffff`

### Status Queries
- [ ] `getStatus()`
- [ ] `isDiscPresent()`
- [ ] `getFullOperatingStatus()`
- [ ] `getOperatingStatus()`
- [ ] `_getPlaybackStatus(p1, p2)`
- [ ] `getPlaybackStatus1()` / `getPlaybackStatus2()`
- [ ] `getPosition()` — track position in `[track, hour, minute, second, frame]`

### Playback Control
- [ ] `_play(action)` — `18c3 ff {action} 000000`
- [ ] `play()` / `pause()` / `fast_forward()` / `rewind()`
- [ ] `stop()` — `18c5 ff 00000000`
- [ ] `gotoTrack(track)` — `1850 ff010000 0000 {track}`
- [ ] `gotoTime(track, h, m, s, f)` — `1850 ff000000 0000 {track} {h}{m}{s}{f}`
- [ ] `_trackChange(direction)` — `1850 ff10 00000000 {direction}`
- [ ] `nextTrack()` / `previousTrack()` / `restartTrack()`

### Disc Operations
- [ ] `ejectDisc()` / `canEjectDisc()` — `18c1 ff 6000`
- [ ] `eraseDisc()` — `1840 ff 0000`
- [ ] `eraseTrack(track)` — `1840 ff01 00 201001 {track}`
- [ ] `moveTrack(source, dest)` — `1843 ff00 00 201001 {s} 201001 {d}`

### Metadata Reads
- [x] `getTrackCount()` — raw query ported, wrapper not
- [x] `_getDiscTitle(wchar)` — raw query ported, only first-chunk path tested
- [ ] `getDiscTitle(wchar)` — wrapper with open/close (partially ported)
- [ ] `getTrackGroupList()` — group structure parsing
- [ ] `getTrackTitle(track, wchar)`
- [ ] `getDiscFlags()` — `1806 01101000 ff00 0001000b`
- [ ] `getDiscCapacity()` — `1806 02101000 3080 0300 ff00 00000000`
- [ ] `_getTrackInfo(track, p1, p2)`
- [ ] `getTrackLength(track)`
- [ ] `getTrackEncoding(track)`
- [ ] `getTrackFlags(track)`
- [ ] `getRecordingParameters()`

### Metadata Writes
- [ ] `setDiscTitle(title, wchar)` — `1807 02201801 00{wc} 3000 0a00 5000 {new} 0000 {old} {sjis}`
- [ ] `setTrackTitle(track, title, wchar)` — `1807 022018{type} {track} 3000 0a00 5000 {new} 0000 {old} {sjis}`

### Secure Upload/Download (`1800 080046 f0030103`)
All commands use the `1800 080046 f0030103` prefix.
- [ ] `saveTrackToArray(track, callback?)`
- [ ] `disableNewTrackProtection(val)`
- [ ] `enterSecureSession()`
- [ ] `leaveSecureSession()`
- [ ] `getLeafID()`
- [ ] `sendKeyData(ekbid, keychain, depth, ekbsignature)`
- [ ] `sessionKeyExchange(hostnonce)`
- [ ] `sessionKeyForget()`
- [ ] `setupDownload(contentid, keyenckey, hexSessionKey)`
- [ ] `commitTrack(tracknum, hexSessionKey)`
- [ ] `sendTrack(wireformat, discformat, frames, pktSize, packets, hexSessionKey, cb?)`
- [ ] `getTrackUUID(track)`
- [ ] `terminate()`

### HiMD Mode (`1800 080046 f0030104`)
- [ ] `enterHiMDMode()`

### Crypto
- [ ] `retailmac(key, value, iv?)` — retail MAC computation for authentication

---

## Factory Interface (factory/netmd-factory-interface.ts)

### Enums
- [ ] `MemoryType` — `MAPPED(0), EEPROM_2(2), EEPROM_3(3)`
- [ ] `MemoryOpenType` — `CLOSE(0), READ(1), WRITE(2), READ_WRITE(3)`
- [ ] `DisplayMode` — `DEFAULT(0), OVERRIDE(1)`

### NetMDFactoryInterface Methods
- [ ] `auth()` — `1801 ff0e 4e6574204d442057616c6b6d616e` ("Net MD Walkman")
- [ ] `changeMemoryState(addr, len, type, state, enc?)`
- [ ] `read(addr, len, type)` — `1821 ff`
- [ ] `write(addr, data, type)` — `1822 ff`
- [ ] `readMetadataPeripheral(sector, offset, len)` — `1824 ff`
- [ ] `writeMetadataPeripheral(sector, offset, data)` — `1825 ff`
- [ ] `setDisplayMode(mode)`
- [ ] `setDisplayOverride(text, blink)`
- [ ] `getDeviceVersion()` — `1813 ff`
- [ ] `getDeviceCode()` — `1812 ff` (chip type, hwid, version, subversion)
- [ ] `getSwitchStatus()` — `1853 ff`

### HiMDFactoryInterface (extends NetMDFactoryInterface)
- [ ] `auth()` — `1802 ff04 4d44574d` ("MDWM")
- [ ] `changeMemoryState()` — `182b ff` (different opcode, length encoded in type field)
- [ ] `read()` — `182c ff` (max 0x1F bytes per read)
- [ ] `write()` — `182d ff` (max 0x1F bytes per write)

### RH1FactoryInterface (extends HiMDFactoryInterface)
- [ ] `changeMemoryState()` — stub (MZ-RH1 DRAM-based, no EEPROM)
- [ ] `translateToDRAMAddr(address)` — address translation logic
- [ ] `read()` / `write()` — delegated to metadata peripheral

---

## Factory Commands (factory/netmd-factory-commands.ts)

- [ ] `PatchPeripheralBase` enum — `NETMD(0x03802000)`, `HIMD(0x03804000)`
- [ ] `display(factoryInterface, text, blink?)` — set display with text sanitization
- [ ] `cleanRead(factoryInterface, address, length, type, enc?, autoDecrypt?)`
- [ ] `cleanWrite(factoryInterface, address, data, type, enc?, autoEncrypt?)`
- [ ] `writeOfAnyLength(factoryInterface, address, data, type, enc?)` — chunked writes (0x10 blocks)
- [ ] `readPatch(factoryInterface, patchNumber, peripheralBase?)`
- [ ] `patch(factoryInterface, address, value, patchNumber, totalPatches, peripheralBase?)`
- [ ] `unpatch(factoryInterface, patchNumber, totalPatches, peripheralBase?)`
- [ ] `readUTOCSector(factoryInterface, sector)` — 2352-byte UTOC read
- [ ] `writeUTOCSector(factoryInterface, sector, data)` — 2352-byte UTOC write
- [ ] `getDescriptiveDeviceCode(factoryInterface)` — chip code string (R, S, Hp, Hn, Hr, Hx)
- [ ] `calculateEEPROMChecksum(data, isHiMD)` — CRC-16-CCITT

---

## Encryption & EKB (netmd-ekb.ts, encrypt-generator.ts)

- [ ] `EKB` trait — `getRootKey()`, `getEKBID()`, `getEKBDataForLeafId()`
- [ ] `EKBOpenSource` — hardcoded open-source key
- [ ] `CorruptedDeckEKB` — device-specific key (MDS-JB980/JE780/NT1)
- [ ] `getEKBForDevice(leafID, vid, pid)` — EKB selection logic
- [ ] `getAsyncPacketIterator({data, frameSize, kek, chunkSize})` — DES-CBC encryption chunk generator

---

## High-Level Commands (netmd-commands.ts)

### Data Structures (interfaces → structs)
- [ ] `Device` — manufacturerName, productName, vendorId, productId, name
- [ ] `Track` — index, title, fullWidthTitle, duration, channel, encoding, protected
- [ ] `Group` — index, title, fullWidthTitle, tracks
- [ ] `Disc` — title, fullWidthTitle, writable, writeProtected, used, left, total, trackCount, groups
- [ ] `DeviceStatus` — discPresent, time, track, state

### Disc Content
- [ ] `listContent(mdIface)` — full disc enumeration (title, groups, tracks, capacity)
- [ ] `getDeviceStatus(mdIface)` — comprehensive status snapshot
- [ ] `countTracksInDisc(disc)` / `getTracks(disc)` — helper accessors

### Title Management
- [ ] `getCellsForTitle(trk)` — TOC cell allocation calculation
- [ ] `getRemainingCharactersForTitles(disc, includeGroups?)`
- [ ] `compileDiscTitles(disc)` — cell allocation for all titles
- [ ] `rewriteDiscGroups(mdIface, disc)` — rewrite group structure
- [ ] `renameDisc(mdIface, newName, newFullWidthName?)`

### Upload/Download
- [ ] `upload(mdIface, track, progressCallback?)` — read track from device (`[DiscFormat, Uint8Array]`)
- [ ] `prepareDownload(mdIface)` — acquire + disable new track protection
- [ ] `download(mdIface, track, progressCallback?)` — write MDTrack to device
- [ ] `formatToHiMD(mdIface)` — erase disc + enter HiMD mode

### Device Discovery
- [ ] `openPairedDevice(usb, logger?)` / `openNewDevice(usb, logger?)`
- [ ] `listDevice(usb)` — device metadata

---

## MDTrack & MDSession (netmd-interface.ts:944–1153)

- [ ] `MDTrack` class — audio data container with title, format, frame info, packet iteration
  - `getTitle()` / `getFullWidthTitle()` / `getDataFormat()` / `getFrameCount()` / `getFrameSize()`
  - `getContentID()` — hardcoded 20-byte content ID
  - `getKEK()` — hardcoded 8-byte KEK
  - `getPacketIterator()` — async packet generation (DES encrypt per chunk)
- [ ] `MDSession` class — secure download session lifecycle
  - `init()` — enter secure session, exchange keys, compute session key
  - `downloadTrack(trk, progressCallback?, discFormat?)`
  - `close()` — forget session key, leave secure session

---

## Utility Functions (utils.ts)

- [x] `parse_u16(buf)`, `parse_u8(buf)` — big-endian byte parsing
- [x] `parse_string(buf)` — SHIFT_JIS decoding
- [ ] `sleep(msec)` — async delay
- [ ] `formatTimeFromFrames(value)` — frame count → `HH:MM:SS+FFF`
- [ ] `timeToFrames([h,m,s,f])` — time array → absolute frame count
- [ ] `encodeToSJIS(utf8)` / `decodeFromSJIS(sjis)` — text encoding
- [ ] `getLengthAfterEncodingToSJIS(utf8)` — byte length prediction
- [x] `getHalfWidthTitleLength(title)` — title length with multi-byte awareness
- [x] `sanitizeHalfWidthTitle(title)` — full-width → half-width conversion
- [x] `sanitizeFullWidthTitle(title)` — half-width → full-width + JP/RU/DE mappings
- [ ] `sanitizeTrackTitle(title)` — encodeURIComponent wrapper
- [ ] `aggressiveSanitizeTitle(title)` — strip non-ASCII
- [x] `halfWidthToFullWidthRange(range)` — ASCII "1-3" → "１－３"
- [ ] `createAeaHeader(...)` — 2048-byte AEA header
- [ ] `createWavHeader(format, bytes)` — WAV/ATRAC3 header
- [ ] `encryptDataForFactoryTransfer(data)` / `decryptDataFromFactoryTransfer(data)` — DES-ECB
- [ ] `concatUint8Arrays(...)` / `concatArrayBuffers(a, b)` — buffer helpers
- [ ] `stringToCharCodeArray(str)` — string → char codes
- [ ] `hexEncode(str)` — hex encode string
- [ ] `pad(str, pad)` — left-padding
- [ ] `assert*` type guards — `assert`, `assertBigInt`, `assertNumber`, `assertString`, `assertUint8Array`

---

## Error Types (netmd-shared-objects.ts)

- [ ] `NetMDError` — base error
- [ ] `NetMDNotImplemented` — command not implemented (status 0x08)
- [ ] `NetMDRejected` — command rejected (status 0x0a)
- [ ] Status code → error mapping in `readReply`

---

## Worker Threads (node-encrypt-worker.ts, web-encrypt-worker.ts)

- [ ] `makeGetAsyncPacketIteratorOnWorkerThread(worker, progressCallback?)` — offloads DES encryption to worker thread. Low priority for native Rust (no JS event loop equivalent needed).

---

## CLI (cli.ts)

- [ ] `devices` — list devices
- [ ] `status [readIntervalMS]` — live status display
- [ ] `command [cmd]` — play/stop/next/prev/eject
- [ ] `goto [track]` — seek to track
- [ ] `ll [hex]` — raw hex query
- [ ] `wipe` — erase disc
- [ ] `ls` — list disc content
- [ ] `set_raw_title [title]` — set disc title with groups
- [ ] `upload [inputfile]` — upload music to device
- [ ] `download [track] [outputfile]` — download song
- [ ] `rename [track] [title]` — rename track
- [ ] `move [src] [dst]` — move track
- [ ] `read_utoc [outputfile]` — read UTOC
