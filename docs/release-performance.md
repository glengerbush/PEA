# Desktop release caches

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
