# Releasing & the update flow

How a release is cut, what the machinery does, and how each kind of install
receives the update. The short version: **you bump a version, push a tag, and
publish the draft release, everything else is automatic.**

```
 DEV                        CI (GitHub Actions)                USERS
 ───────────────           ─────────────────────────          ─────────────────────────
 bump version      tag     ubuntu-22.04: AppImage/.deb/.rpm   app checks latest.json
 commit + tag  ──push──▶   macos-latest: .dmg (arm64)     ┌─▶ at every launch
                           macos-13:     .dmg (x64)       │   "Update available →
                           sign updater artifacts         │    Update now" → restart
                           draft release + latest.json    │
 publish the draft  ──────────────────────────────────────┘
```

---

## One-time setup (before the first release)

1. **Add the updater signing key as a repo secret.** The private key lives at
   `~/.tauri/openarchiver-updater.key` (no password). Add its *contents* as the
   GitHub Actions secret `TAURI_SIGNING_PRIVATE_KEY`
   (repo → Settings → Secrets and variables → Actions).
2. **Back that key up somewhere safe.** Every update is signed with it and the
   matching public key is baked into every installed app
   (`apps/desktop/src-tauri/tauri.conf.json` → `plugins.updater.pubkey`).
   If the key is lost, existing installs can never auto-update again, users
   would have to manually download and reinstall once.
3. The repo must stay **public**: installed apps fetch
   `https://github.com/glengerbush/PEA/releases/latest/download/latest.json`
   unauthenticated.

---

## Cutting a release (dev responsibility, every time)

1. **Bump the version:** this is the step that actually makes updates happen.
   The updater compares the version *compiled into the app* against the
   version in the published `latest.json`; the git tag is just the trigger and
   the label. Keep all three in sync with the tag:

   | File | Field | Used for |
   |---|---|---|
   | `apps/desktop/src-tauri/tauri.conf.json` | `version` | **authoritative:** what the updater compares |
   | `package.json` (root) | `version` | version shown in the web UI |
   | `packaging/arch/PKGBUILD` | `pkgver` | Arch `makepkg` installs |

2. **Commit, tag, push:**

   ```bash
   git add -A && git commit -m "Release v0.8.0"
   git tag v0.8.0
   git push && git push --tags
   ```

   > Is tagging required? Yes, the release workflow only runs on tags matching
   > `v*` (`.github/workflows/release.yml`). A plain push to main builds nothing.

3. **Wait for CI** (~15–25 min). The workflow builds on three runners
   (Ubuntu 22.04, macOS arm64, macOS Intel), compiles the self-contained
   Rust binary (the whole engine is linked in; only the static frontend ships
   as a resource), signs
   the updater artifacts with the secret key, and attaches everything to a
   **draft** release:

   | Asset | Purpose |
   |---|---|
   | `PEA_X.Y.Z_amd64.AppImage` (+ `.sig`) | Linux portable app, self-updates in place |
   | `PEA_X.Y.Z_amd64.deb` | Debian/Ubuntu package (also the PKGBUILD source) |
   | `PEA-X.Y.Z-1.x86_64.rpm` | Fedora package |
   | `PEA_X.Y.Z_{aarch64,x64}.dmg` + `.app.tar.gz` (+ `.sig`) | macOS installs and update payloads |
   | `latest.json` | the update feed: version + per-platform URLs + signatures |

4. **Review and publish the draft.** This is deliberate: nothing reaches users
   until you click *Publish release*. The updater endpoint
   (`releases/latest/download/latest.json`) only resolves for the latest
   **published** release, drafts and pre-releases are invisible to it.

5. Sanity check: `curl -sL https://github.com/glengerbush/PEA/releases/latest/download/latest.json | head`
   should show the new version.

**Common pitfall:** tagging without bumping `tauri.conf.json` produces a
release whose `latest.json` still carries the old version, installed apps
will correctly conclude there is nothing to update.

---

## How each kind of install gets the update

On every launch the desktop app fetches `latest.json`, and if its version is
newer than the running app it shows a dialog: **"Update now / Later"**.
Choosing *Update now* downloads the platform artifact, verifies its minisign
signature against the baked-in public key, installs, and restarts the app.
The archive itself (database, emails, search index) lives in the data
directory and is never touched by updates.

| Install type | What the user does | What happens under the hood |
|---|---|---|
| **Linux AppImage** (incl. `scripts/install-desktop.sh` installs) | Click *Update now*. | The AppImage replaces itself in place and relaunches. Fully hands-off. |
| **Linux .deb / .rpm** | Click *Update now*; a privileged package-install prompt may appear (updater plugin ≥2.10 supports native packages). If that flow ever misbehaves, downloading the new .deb/.rpm from the release and installing it is equivalent. | Package manager installs the new version over the old. |
| **Arch (PKGBUILD)** | The in-app dialog will appear but pacman owns the files. Instead update with: `cd packaging/arch && git pull && makepkg -si` (bump `pkgver` if you pinned it). Or switch to the AppImage for fully automatic updates. | pacman replaces the package. |
| **macOS (.dmg install)** | Click *Update now*. | The updater downloads `.app.tar.gz`, swaps `PEA.app`, relaunches. Because the app itself performs the download, Gatekeeper's "unidentified developer" prompt does **not** reappear on updates, it's first-install only (System Settings → Privacy & Security → *Open Anyway*). |

### Trust model, in one paragraph

Updates are authenticated by **signature, not by transport alone**: CI signs
every updater artifact with your private key, `latest.json` carries those
signatures, and each installed app verifies them against the public key
compiled into it. Even someone who could tamper with the GitHub release
cannot push a malicious update without the private key, which is also why
losing that key permanently breaks auto-update for every existing install.

---

## Versioning conventions

- Plain semver, `vMAJOR.MINOR.PATCH` tags (`v0.8.0`).
- The updater does a semver comparison, any higher published version
  triggers the prompt; there is no channel/beta mechanism configured.
- Data-directory compatibility: migrations run automatically at app startup,
  so upgrades are forward-only. Downgrading after a release that shipped a
  schema migration is not supported (restore from a backup instead).
