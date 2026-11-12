## What and why

## Version

Pull requests to `main` (the Swift 1.x line): add exactly one label before merging; a merge to `main` releases automatically. Pull requests to `v2` (Rust) release nothing; use `semver:major` only to switch off the compatibility checks for a deliberate break.

- `semver:major`: a breaking change to `docs/api.md`
- `semver:minor`: new API
- `semver:patch`: fixes and documentation (the default when no label is set)

- [ ] On `main` only: if this changes behaviour of the Swift library, I added a line to `docs/rust-port/parity-ledger.md` on `v2`.
