# Desktop release caches

## Exact-revision release inputs

Main builds now upload the unbundled executable and frontend resources as a
14-day artifact, keyed by commit, OS and architecture. Tagged builds look only in
the main workflow for the exact commit and require the matching platform job to
have succeeded. This works even if the other platform is still queued.

Before restoring files, the release validates revision, version, platform, every
file checksum and allowed archive paths. Missing, expired or unavailable artifacts
fall back to the normal locked build; invalid downloaded artifacts fail the job.
`tauri bundle` packages verified inputs without recompiling. The Tauri release
action still handles updater signatures, assets and draft releases as before.

With verified inputs, setup skips the Rust build cache and frontend build, and
installs runtime libraries rather than compiler headers. In a clean Ubuntu 22.04
dependency-resolution check this reduced newly installed packages from 429 to 273.
Recommended runtime components (graphics, SVG loaders and desktop portals) remain
enabled, matching the normal build's installation policy.
This avoids adding a separately maintained CI image; hosted timing must still
measure the installation gain. The existing Ubuntu baseline is unchanged.

Dispatch `release.yml` manually to benchmark the same packaging/signing path
without publishing anything. It reports whether it reused inputs or compiled, and
elapsed build/bundle time in the job summary. For a warm benchmark, wait for the
matching platform's main preparation job to finish first. No version bump is needed.

Compression was measured locally on the actual 1.2.9 AppImage payload using Ubuntu
22.04 squashfs-tools, two compression workers and 128 KiB blocks: Zstandard took
26.86s and produced 86,691,840 bytes; gzip took 31.21s and produced 94,531,584 bytes.
The existing Zstandard setting is retained. This measures compression alone, not
the full packaging step. Compiler optimization settings are also unchanged.

Verification: artifact round-trip, executable permissions, stale resource cleanup,
revision/version/platform mismatches, corruption, traversal/link rejection, trusted
main job selection and normal-build fallback are covered by Python tests:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s scripts -p 'test_desktop_release.py'
```

An extracted release executable/resources also completed a real local Debian
bundle through the wrapper without compilation (updater signing disabled for this
local smoke check). Hosted benchmarks exercise fresh unbundled inputs and signing.

## Dependency cache fallback

Release tags and the main-branch cache workflow use the same local composite action
to prepare Rust 1.99.1, Node 22, locked pnpm dependencies and frontend resources.
The Cargo cache covers the root workspace's `target` directory, with separate keys
for OS and architecture. Dependency/toolchain changes invalidate the corresponding
cache keys. Release optimization settings and all Linux/macOS package formats are
unchanged; tagged builds still create signed updater artifacts and draft releases.

GitHub caches belong to a branch or tag. A new tag can restore `main` caches, but
cannot restore another tag's cache. `Prepare desktop release caches` therefore
compiles on relevant pushes to `main`, without bundling, signing or publishing.
Only main saves the shared cache. Superseded warm-up runs are canceled; release
builds are not. This moves compilation work ahead of releases and adds build work
to relevant main-branch pushes.

For the first release after installing these changes, let the main-branch warm-up
finish before pushing the release tag. It can also be dispatched manually on
`main`. Matching caches can be evicted; cold releases still rebuild normally.

## Hosted measurement — 2026-11-13

Baseline: recent releases took 8m07s–9m29s. The previous
[1.2.8 run](https://github.com/glengerbush/PEA/actions/runs/31238977337)
took 8m07s overall; its Linux job took 8m03s and macOS job 6m55s.

The first [main-branch warm-up](https://github.com/glengerbush/PEA/actions/runs/43464646948)
completed its Linux job in **8m24s**, including **6m24s** compiling from cold and
saving a roughly 641 MB cache. macOS remained queued without a runner for over nine
minutes. The remaining warm-up was canceled after Linux saved its cache, and
`v1.2.9` was then tagged. This warm-up is additional CI work, not part of the tagged
release timing. macOS has no warmed cache from this run.

The [1.2.9 release run](https://github.com/glengerbush/PEA/actions/runs/43466232542)
successfully restored that cache. Its Linux job succeeded in **8m35s**, versus
8m03s previously: **32 seconds / 7% slower overall**. This run does not demonstrate
an end-to-end release speed improvement.

| Measurement                                 | Baseline                         | 1.2.9 |
| ------------------------------------------- | -------------------------------- | ----- |
| Linux job                                   | 8m03s (1.2.8)                    | 8m35s |
| Tauri compilation, bundling and upload step | 6m55s (1.2.8)                    | 5m49s |
| Rust compilation alone                      | 6m24s (same-commit cold warm-up) | 2m57s |

The cache cut compilation by 54% against the same-day cold build, but setup took
2m28s and bundling took about 2m38s, mostly AppImage packaging. The cache restore
itself took 21s. These are individual hosted observations, not controlled repeated
benchmarks; runner/setup variation can outweigh the compilation gain.

As of 2026-11-13 16:56 UTC, macOS remained queued without a runner. The release run
was left active. The draft contains Linux deb/rpm/AppImage packages, their updater
signatures and `latest.json`; no macOS result or complete-release duration is yet
available. The existing draft publication policy is unchanged.

Validation also covers workflow syntax/formatting, matching platform/cache
configuration, publication gates, root Cargo workspace metadata and the installed
Tauri CLI's `--no-bundle` and Cargo argument support.
