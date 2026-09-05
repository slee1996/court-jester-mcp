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

For version `0.2.16`:

```bash
git tag -a v0.2.16 -m "Release 0.2.16"
git push origin main
git push origin v0.2.16
```

Pushing the tag starts `.github/workflows/release.yml`. The workflow:

1. runs the reusable Ubuntu quality workflow against the tag, including sample verification and the release-binary repair contract; it does not repeat the complete local package-staging recipe;
2. independently checks that the tag exactly matches the Cargo package version and dated release notes in each platform build;
3. builds macOS and Linux archives for Arm64 and AMD64;
4. emits a SHA-256 file beside every archive and verifies every checksum;
5. creates one GitHub release from `docs/release-notes-0.2.16.md` and uploads all eight files.

Do not create the tag until the intended commit is on `main`. The workflow uses `--verify-tag` and will reject a missing or mismatched tag.

## Current-build repair evidence

Quality runs on pull requests, direct pushes to `main`, and release workflow calls. It installs Node 24 explicitly, runs the Rust suite and maintained Python contract/benchmark tests, builds the release binary, and exercises sample verification plus the deterministic repair contract.

Use `just repair-check`, or:

```bash
python3 scripts/check_repair_loop.py --binary target/release/court-jester
```

An optional `--output <new-file>` writes artifact-v1 JSON without overwriting an existing file. The four cases cover Python/TypeScript runtime and declared-property failures. Each checks original detection/replay, rejection of a different error or skipped oracle, replay after a supplied repair, and exported-test behavior against the repair, false repair, and original bug. The artifact records phase timings, fixture digest, binary version and SHA-256 before/after, and separate contract/protocol/inconclusive/launch/timeout failure causes. Changing the binary mid-run or supplying an empty suite cannot produce success.

Each supplied repair also runs three independent public fixture examples, with exact output and input-preservation expectations retained in the artifact. Property repairs sort the actual input rather than returning a constant sorted array. These 12 examples supplement the recorded counterexample check; they are neither held-out cases nor proof of general correctness. They prevent the gate from treating a witness-only stand-in as its reference repair.

CI uploads `target/repair-contract.json` when produced, including failed-case evidence, under a run/attempt-specific artifact name. This is a current-binary contract check using deterministic repairs, **not** an agent benchmark, precision/recall estimate, or proof for every release platform. Hosted CI execution, platform-specific binaries, broader held-out evidence, and real user outcomes remain separate acceptance requirements. No release is published merely by running this check or pushing to `main`.

## Verify the published release

Watch the release workflow and inspect the resulting release:

```bash
gh run list --workflow Release --limit 1
gh run watch <run-id>
gh release view v0.2.16 --json tagName,name,isDraft,isPrerelease,assets,url
```

The release must be non-draft and non-prerelease and contain these platform archives plus matching `.sha256` files:

- `court-jester-v0.2.16-darwin-arm64.tar.gz`
- `court-jester-v0.2.16-darwin-amd64.tar.gz`
- `court-jester-v0.2.16-linux-arm64.tar.gz`
- `court-jester-v0.2.16-linux-amd64.tar.gz`
- one matching `.sha256` file for each archive.

Finally, run the documented installer on a disposable account or machine and confirm `court-jester --version` prints `court-jester 0.2.16`. The installer downloads the matching checksum and refuses an archive it cannot verify.

## Backfill

The manual `workflow_dispatch` path accepts an existing `v0.2.16` tag. It is for rebuilding assets from that immutable tag, not for releasing an uncommitted worktree or publishing to crates.io.
