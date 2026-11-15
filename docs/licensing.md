# Licensing

This repository is MIT (`LICENSE`). The crates have no dependency under MPL-2.0 or any other copyleft licence: `cargo deny list` and `cargo deny check licenses` (checked 2026-11-15, Cargo.lock at the 2.0.1 release) show only MIT, Apache-2.0, Unicode-3.0 and Unlicense, alone or as a choice, and `deny.toml` allows nothing else. `r-efi` offers `LGPL-2.1-or-later` as one option of three and is used under MIT or Apache-2.0.

The library ships as source (a git tag); it publishes no binary and no package. A product that links it into a binary reproduces the notices of the crates it links. `timetug-shared` does that: its `THIRD-PARTY-NOTICES.md` lists every crate of the connectors with its licence text, and ships in each artifact built from them.

## UniFFI and the MPL-2.0

`uniffi` (MPL-2.0) is not a dependency here. It is a dependency of `timetug-shared`'s bindings crate (`timetug-engine-ffi`), which links this library. The owner accepted the UniFFI licence on 2026-11-15; what it requires and where each requirement is met is in [docs/licensing.md of timetug-shared](https://github.com/binary-companion/timetug-shared/blob/main/docs/licensing.md). MPL-2.0 is file-level: it covers the UniFFI files, not these crates, which stay MIT.

Adding a dependency under MPL-2.0 or another copyleft licence here is a new owner decision: `deny.toml` has no exception for any.
