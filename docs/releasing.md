# Releasing Court Jester

Court Jester currently ships through GitHub Releases only. Do not publish this repository to crates.io.

## Prepare

1. Confirm `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, and `docs/release-notes-<version>.md` agree on the release version.
2. Run the complete local gate:

   ```bash
   just release-check
   ```

   If `just` is unavailable, run the commands in the `release-check` recipe directly. Use the rustup toolchain pinned by `rust-toolchain.toml` rather than an older system Cargo.
3. Review `git diff --check`, `git status --short`, and the final diff. Commit every intended source, test, documentation, workflow, and release-metadata change. Keep generated `target/`, benchmark results, and local governed-work artifacts out of the commit.

## Publish

For version `0.2.9`:

```bash
git tag -a v0.2.9 -m "Release 0.2.9"
git push origin main
git push origin v0.2.9
```

Pushing the tag starts `.github/workflows/release.yml`. The workflow:

1. reruns the reusable formatting, Clippy, Rust test, benchmark unit, release-contract, release-build, smoke, and locked dry-run gates against the tag;
2. checks that the tag exactly matches the Cargo package version and dated release notes;
3. builds macOS and Linux archives for Arm64 and AMD64;
4. emits a SHA-256 file beside every archive and verifies every checksum;
5. creates one GitHub release from `docs/release-notes-0.2.9.md` and uploads all eight files.

Do not create the tag until the intended commit is on `main`. The workflow uses `--verify-tag` and will reject a missing or mismatched tag.

## Verify the published release

Watch the release workflow and inspect the resulting release:

```bash
gh run list --workflow Release --limit 1
gh run watch <run-id>
gh release view v0.2.9 --json tagName,name,isDraft,isPrerelease,assets,url
```

The release must be non-draft and non-prerelease and contain these platform archives plus matching `.sha256` files:

- `court-jester-v0.2.9-darwin-arm64.tar.gz`
- `court-jester-v0.2.9-darwin-amd64.tar.gz`
- `court-jester-v0.2.9-linux-arm64.tar.gz`
- `court-jester-v0.2.9-linux-amd64.tar.gz`
- one matching `.sha256` file for each archive.

Finally, run the documented installer on a disposable account or machine and confirm `court-jester --version` prints `court-jester 0.2.9`. The installer downloads the matching checksum and refuses an archive it cannot verify.

## Backfill

The manual `workflow_dispatch` path accepts an existing `v0.2.9`-or-newer tag. It is for rebuilding assets from that immutable tag, not for releasing an uncommitted worktree or publishing to crates.io.
