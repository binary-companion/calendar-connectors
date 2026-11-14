## What and why

## Version

Pull requests to `main` (the Rust library, 2.x) and to `1.x` (the Swift line, kept for the Mac until it moves to the Rust engine): add exactly one label before merging; a merge to either branch releases automatically.

- `semver:major`: a breaking change to `docs/api.md`
- `semver:minor`: new API
- `semver:patch`: fixes and documentation (the default when no label is set)

- [ ] On `1.x` only: if this changes behaviour of the Swift library, the same change is made (or a pull request is open) on `main` in Rust.
