# AGENTS.md

Rust workspace (resolver 3) for talking to Sony/Sharp NetMD MiniDisc devices over USB.
Three crates under `crates/`: `netmd` (protocol lib), `minidisc-audio` (decode/resample/encode), `minidisc-cli` (binary).

## Build / test gotchas

- **Package vs binary name mismatch**: the package is `minidisc-cli`; clap's `name = "rmd"` only sets the help/usage string. Use `cargo build -p minidisc-cli` / `cargo run -p minidisc-cli -- <args>`. `-p rmd` does NOT work (README's `-p rmd` is stale).
- Run all tests: `cargo test` (unit tests are inline in `crates/*/src/*.rs`; they are parser/scan/crypto tests and need **no** hardware). Single crate: `cargo test -p netmd`.
- Actually exercising device commands requires a real USB NetMD recorder; there is no hardware mock.

## Reference ports (gitignored, not part of the build)

`netmd-js/` (TypeScript) and `webminidisc/` are the upstream JS implementations being ported, kept locally for reference and `.gitignore`d. Do not edit them or treat them as build inputs.

## Porting workflow

This repo is a line-by-line port of `netmd-js`. `PORTING_REFERENCE.md` is the authoritative map. Key rules when porting a command from JS:

- Rust hex query strings include the leading `00` status byte; JS prepends it at send time. Add `00 ` to JS query strings.
- Rust `scan` templates start with `%?` to consume the status byte the JS reply strips. Add `%? ` to JS scan templates.
- Always open the relevant descriptor TD before a read and close it after (see PORTING_REFERENCE.md table). Double-check the close descriptor.
- `OperatingStatusBlock` descriptor is `80 00`, not `00 00`.

`UNPORTED.md` / `TODO.md` / `CORE_TODO.md` track remaining work.

## Architecture notes

- `netmd/src/lib.rs` is a thin facade re-exporting from protocol-area modules (`disc`, `track_info`, `playback`, `secure`, `groups`, `transport`, etc.). Add new APIs in the relevant module and re-export there.
- USB I/O lives in `transport.rs` (`send_query`, `read_reply*`); reply parsing uses the `scan`/`query` template language documented in PORTING_REFERENCE.md, with helpers in `util.rs`.
- Audio upload path (`minidisc-audio`): non-ATRAC3 input is decoded with `symphonia`, resampled to 44.1 kHz stereo with `rubato`; SP -> s16be PCM, LP2/LP4 -> WAV then encoded via the external `atracdenc` crate. ATRAC3 `.wav` files upload directly without transcoding.
</content>
</invoke>
