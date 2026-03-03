# Release Guide

This project uses tag-based automated releases.

## Prerequisites

- GitHub repository secrets:
  - `CARGO_REGISTRY_TOKEN` for crates.io publishing
  - `HOMEBREW_TAP_TOKEN` for pushing formula updates to `holon-run/homebrew-tap`
- crates.io package name is available (`uxc`)

## Pre-release Checklist

1. Ensure your working tree is clean.
2. Update version in `Cargo.toml` and `Cargo.lock`.
3. Move release notes from `CHANGELOG.md` `Unreleased` to `## [x.y.z]`.
4. Run local verification:

```bash
./scripts/release-check.sh vX.Y.Z
```

5. Commit and merge to `main`.

## Trigger a Release

Create and push a tag:

```bash
git tag vX.Y.Z
git push origin vX.Y.Z
```

`Release` workflow will:

1. Validate tag/version/changelog consistency
2. Build and package binaries for:
   - `x86_64-unknown-linux-gnu`
   - `aarch64-unknown-linux-gnu`
   - `x86_64-unknown-linux-musl`
   - `x86_64-apple-darwin`
   - `aarch64-apple-darwin`
3. Generate `uxc-vX.Y.Z-checksums.txt`
4. Create GitHub Release with all assets
5. Publish crate to crates.io
6. Update `holon-run/homebrew-tap` Formula

Windows users should run UXC through WSL.

## Rollback

If release failed after tag push:

1. Fix issue on a branch and merge to `main`.
2. Delete broken tag from remote:

```bash
git push --delete origin vX.Y.Z
git tag -d vX.Y.Z
```

3. Create a new tag (recommended: bump patch version).

If crate was already published, version cannot be reused. Publish a new version.

## Troubleshooting

- `cargo publish` failure:
  - verify `CARGO_REGISTRY_TOKEN`
  - ensure version is not already published
- Homebrew update skipped:
  - check `HOMEBREW_TAP_TOKEN` secret exists
  - check token has push permission to `holon-run/homebrew-tap`
- Missing release assets:
  - inspect failed matrix build job for target-specific toolchain errors
