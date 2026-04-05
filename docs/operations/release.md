# Release Guide

This project uses tag-based automated releases.

## Prerequisites

- GitHub repository secrets:
  - `CARGO_REGISTRY_TOKEN` for crates.io publishing
  - `HOMEBREW_TAP_TOKEN` for pushing formula updates to `holon-run/homebrew-tap`
- crates.io package name is available (`uxc`)
- npm package `@holon-run/uxc-daemon-client` already exists on npm
- npm Trusted Publishing is configured for:
  - repository: `holon-run/uxc`
  - workflow: `.github/workflows/release.yml`

## Pre-release Checklist

1. Ensure your working tree is clean.
2. Update version in `Cargo.toml`, `Cargo.lock`, and `packages/uxc-daemon-client/package.json`.
3. Move release notes from `CHANGELOG.md` `Unreleased` to `## [x.y.z]`.
4. Run local verification:

```bash
./scripts/release-check.sh vX.Y.Z
```

If you intentionally run checks before committing version/changelog changes, use:

```bash
./scripts/release-check.sh vX.Y.Z --allow-dirty
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
6. Publish `@holon-run/uxc-daemon-client` to npm
7. Update `holon-run/homebrew-tap` Formula

The npm publish step uses GitHub Actions Trusted Publishing via OIDC.
It does not require an `NPM_TOKEN`.

Windows users should run UXC through WSL.

## Post-release Checklist

After the workflow creates the GitHub Release, review and update the release page.

Do not keep the default auto-generated PR list as the final release notes.
Replace it with a short curated summary that matches the shipped version:

- highlight the main user-facing changes
- keep the scope aligned with the actual tagged release
- include install commands for the tagged version when helpful

Typical command flow:

```bash
gh release view vX.Y.Z
gh release edit vX.Y.Z --notes-file /path/to/release-notes.md
```

For patch releases, keep the notes short and focused.
For feature releases, summarize the product direction as well as the main technical additions.

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
- npm publish failure:
  - verify npm Trusted Publishing is configured for `@holon-run/uxc-daemon-client`
  - verify the workflow filename matches `.github/workflows/release.yml`
  - verify the workflow still has `permissions: id-token: write`
- Missing release assets:
  - inspect failed matrix build job for target-specific toolchain errors
