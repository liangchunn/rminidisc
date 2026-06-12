# NetMD Rust ↔ JS Porting Reference

## USB Transfer Pattern

Both implementations use USB control transfers for command/response.

| | JS (`netmd.ts`) | Rust (`main.rs`) |
|---|---|---|
| Command request | `controlTransferOut(0x80, ...)` | `write_control(0x80, ...)` |
| Factory command request | `controlTransferOut(0xff, ...)` | not ported yet |
| Reply length | `controlTransferIn(0x01, 4)` | `read_control(0x01, 4)` |
| Reply data | `controlTransferIn(0x81, len)` | `read_control(0x81, len)` |

## Status Byte Handling

The JS and Rust implementations differ in where the `0x00` status byte is injected.

**JS approach:**
- Format queries **without** the status byte: `"1808 10 1001 01 00"`
- `sendCommand()` prepends `[0x00]` before sending via USB

**Rust approach:**
- Format queries **with** the status byte: `"00 1808 10 1001 01 00"`
- Sent directly via `write_control`

The wire bytes are identical either way. When porting a query string from JS to Rust, **prefix it with `00`**. When comparing scan templates, the Rust version will have an extra leading `%?` to consume the status byte in the reply.

## Reply Handling

**JS (`netmd-interface.ts:229-259`):**
- Reads full reply data (status + payload) via `netMd.readReply()`
- Inspects status byte, throws on error
- Returns `data.buffer.slice(1)` — **payload only, status byte stripped**

**Rust (`main.rs:115-154`):**
- Reads 4-byte header via `read_control(0x01, ...)`
- Uses `header[2]` as payload length
- Reads payload via `read_control(0x81, ...)`
- Returns payload as-is (**includes status byte**)

This means Rust scan templates start with `%?` to skip the status byte, while JS templates do not.

### Example: Track Count

```
JS send:  1806 02101001 3000 1000 ff00 00000000
Rust send: 00 1806 02101001 3000 1000 ff00 00000000

JS scan:  1806 02101001 %?%? %?%? 1000 00%?0000 0006 0010000200%b
Rust scan: %? 1806 02101001 %?%? %?%? 1000 00%?0000 0006 0010000200%b
```

## Descriptor Enum

| JS (`netmd-interface.ts`) | Rust (`descriptor.rs`) | Hex Value |
|---|---|---|
| `discTitleTD` | `DiskTitleTd` | `10 1801` |
| `audioUTOC1TD` | `AudioUtoc1Td` | `10 1802` |
| `audioUTOC4TD` | `AudioUtoc4Td` | `10 1803` |
| `DSITD` | `DsiTd` | `10 1804` |
| `audioContentsTD` | `AudioContentsTd` | `10 1001` |
| `rootTD` | `RootTd` | `10 1000` |
| `discSubunitIndentifier` | `DiscSubUnitIdentifier` | `00` |
| `operatingStatusBlock` | `OperatingStatusBlock` | **`80 00`** (not `00 00`) |

## DescriptorAction Enum

| JS | Rust | Hex |
|---|---|---|
| `openRead` | `OpenRead` | `01` |
| `openWrite` | `OpenWrite` | `03` |
| `close` | `Close` | `00` |

## Descriptor Command Frame

```
JS:   1808 {descriptor} {action} 00
Rust: 00 1808 {descriptor} {action} 00
```

Both produce identical USB bytes. The Rust version includes the `0x00` status byte in the formatted string.

## Scan Format Directives

Both implementations share the same pattern language:

| Directive | Meaning | Bytes |
|---|---|---|
| `%?` | Skip/wildcard | 1 |
| `%b` | Unsigned byte | 1 |
| `%w` | Unsigned word (big-endian) | 2 |
| `%d` | Unsigned doubleword (big-endian) | 4 |
| `%q` | Unsigned quadword (big-endian) | 8 |
| `%B` | BCD-encoded byte | 1 |
| `%W` | BCD-encoded word | 2 |
| `%x` | Length-prefixed data (2-byte length) | variable |
| `%s` | Length-prefixed null-terminated string | variable |
| `%z` | Length-prefixed data (1-byte length) | variable |
| `%*` | Rest of data | remaining |
| `%#` | All remaining data (non-consuming) | — |

Endianness overrides: `%<d` (little-endian), `%>w` (big-endian). Default is big-endian.

## Implemented Commands (Rust)

### Disc Title Read

```
Send: 00 1806 02201801 00{WC} 3000 0a00 ff00 {remaining:04x}{done:04x}

First chunk scan:  %? 1806 02201801 00%? 3000 0a00 1000 %w0000 %?%?000a %w %*
Subsequent scan:    %? 1806 02201801 00%? 3000 0a00 1000 %w%?%? %*
```

- `WC` = wchar flag (0 or 1)
- `remaining`/`done` track bytes remaining and bytes already consumed

### Track Count

```
Send: 00 1806 02101001 3000 1000 ff00 00000000
Scan: %? 1806 02101001 %?%? %?%? 1000 00%?0000 0006 0010000200%b
```

### Implicit Descriptor State Management

Before reading data, the JS reference **always** opens and closes the relevant descriptor TD:

| Operation | Open | Close |
|---|---|---|
| Read disc title | AudioContentsTd + DiskTitleTd | DiskTitleTd + AudioContentsTd |
| Read track count | AudioContentsTd | AudioContentsTd |

## Not Yet Ported (JS → Rust)

The following command groups exist in `netmd-interface.ts` but are not yet implemented in Rust:

### Playback Control
- `acquire` / `release` — device lock/unlock
- `_play` / `stop` — playback control
- `gotoTrack` / `gotoTime` — seeking
- `_trackChange` — next/prev/restart track
- `ejectDisc` / `canEjectDisc`

### Status Queries
- `getDiscSubunitIdentifier`
- `getStatus` / `getFullOperatingStatus`
- `_getPlaybackStatus` / `getPosition`

### Disc Operations
- `eraseDisc` / `eraseTrack` / `moveTrack`
- `getDiscFlags` / `getDiscCapacity`

### Track Info
- `getTrackTitle` / `setTrackTitle` / `setDiscTitle`
- `_getTrackInfo` / `getTrackLength` / `getTrackEncoding` / `getTrackFlags`
- `getRecordingParameters`

### Secure Upload/Download (`1800 080046 f0030103`)
- `saveTrackToArray` / `sendTrack` / `commitTrack`
- `disableNewTrackProtection`
- `enterSecureSession` / `leaveSecureSession`
- `sendKeyData` / `sessionKeyExchange` / `sessionKeyForget`
- `setupDownload` / `getTrackUUID`
- `terminate`

### HiMD Mode (`1800 080046 f0030104`)
- `enterHiMDMode` / `getLeafID`

## Factory Command Set

JS reference (`netmd-factory-interface.ts`). Not yet ported to Rust.

| Command | Hex |
|---|---|
| Auth (NetMD) | `1801 ff0e 4e6574204d442057616c6b6d616e` |
| Auth (HiMD) | `1802 ff04 4d44574d` |
| Get device version | `1813 ff` |
| Get device code | `1812 ff` |
| Change memory state | `1820 ff ...` / `182b ff ...` |
| Read memory | `1821 ff ...` / `182c ff ...` |
| Write memory | `1822 ff ...` / `182d ff ...` |
| Read metadata peripheral | `1824 ff ...` |
| Write metadata peripheral | `1825 ff ...` |
| Set display mode | `1851 ff ...` |
| Set display override | `1852 ff ...` |
| Get switch status | `1853 ff` |

Factory commands use request code `0xff` instead of normal `0x80`.

## Porting Checklist

When porting a command from JS:

1. Copy the `formatQuery(...)` string from JS → add a leading `00 ` to create the Rust hex string
2. Copy the `scanQuery(...)` template from JS → add a leading `%? ` for the status byte
3. Use `send_query(handle, hex_string)` for commands, `reply.scan(template)` for parsing
4. Ensure descriptor TD is opened before and closed after data reads
5. Use the correct descriptor for close (double-check, do not copy-paste blindly)
6. Parse returned slices with `parse_u8(slice)`, `parse_u16(slice)`, or `parse_string(slice)` from `util.rs`

## Protocol Reply Status Codes

| Code | Status | Meaning |
|---|---|---|
| `0x00` | Control | Request status |
| `0x01` | Status | — |
| `0x02` | SpecificInquiry | — |
| `0x03` | Notify | — |
| `0x04` | GeneralInquiry | — |
| `0x08` | NotImplemented | Command not supported |
| `0x09` | Accepted | Command succeeded |
| `0x0a` | Rejected | Command rejected |
| `0x0b` | InTransition | Device busy |
| `0x0c` | Implemented | — |
| `0x0d` | Changed | — |
| `0x0f` | Interim | Response not ready, retry |
