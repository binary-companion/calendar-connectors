# Rust port

This directory holds the working documents of the Rust port of `calendar-connectors` on the `v2` branch.
`v2` is Rust-only since the change that removed the Swift package (Package.swift, Sources/, Tests/, the Swift CI jobs and the vector exporter). The Swift package stays on `main`, where 1.x is still released; its baseline for the port was tag `1.2.0`.

Last dual-language commit (the last commit of `v2` that still has the Swift package next to the crates): `ea8a9e2fdb29749a3a041ef381eaa1c9aed7f027`. Browse it with `git show ea8a9e2:Package.swift`; check it out to run the Swift tests, regenerate the vectors (`scripts/dev/export-vectors.sh`) or read the Swift sources that `file-map.md` names. No git tag marks it (agents do not tag).

`parity-ledger.md` lists behaviour changes merged to `main` (Swift) after the baseline that Rust must still receive; it has no open entry.
`file-map.md` maps each Swift file to its Rust module; the Swift paths refer to the last dual-language commit.
`swift-tests-baseline.txt` and `test-parity.md` list the Swift tests and the Rust tests that replaced them (checked by `scripts/ci/check-test-parity.py`).
The program tracker is in the `timetug` repository: `docs/superpowers/plans/2026-11-10-rust-engine-program-tracker.md`.
The porting conventions are in the same place: `docs/superpowers/plans/2026-11-10-rust-port-conventions.md`.
