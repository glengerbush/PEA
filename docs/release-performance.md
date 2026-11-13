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

Baseline: recent releases took 8m07s–9m29s. Updated hosted cold/warm timings remain
unmeasured. Validation covers workflow syntax/formatting, matching platform/cache
configuration, publication gates, root Cargo workspace metadata and the installed
Tauri CLI's `--no-bundle` and Cargo argument support. Full macOS/Linux builds and
GitHub cache restore behavior require a hosted run.
