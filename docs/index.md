# PEA (Personal Email Archive)

A local-only desktop app for getting your email out of bad clients and off of
servers, into an archive you can search and organize, completely offline.

One desktop app, one process, zero services. Import your mail once from
`.mbox` or `.eml` files; everything lives on your machine in a single data
directory (`~/.local/share/pea`, macOS: `~/Library/Application Support/PEA`;
the archive index and
full-text search in one SQLite file, next to your email storage.
**Backing up = copying that folder.**

## Installing

### Linux

Use the package for your distro base. The only runtime dependency is
WebKitGTK 4.1, which the native packages take from your system.

**Arch-based** (Arch, Manjaro, EndeavourOS, Omarchy, …) — build the native
package from this repo; it runs against your system WebKitGTK rather than the
AppImage's bundled copy:

```bash
git clone https://github.com/glengerbush/PEA && cd PEA/packaging/arch && makepkg -si
```

Update by re-running `makepkg -si` after a new release.

**Debian/Ubuntu-based** (Debian, Ubuntu, Mint, Pop!\_OS, …) — download the
`.deb` from the [releases page](https://github.com/glengerbush/PEA/releases):

```bash
sudo apt install ./PEA_*_amd64.deb
```

**Fedora/RHEL-based** (Fedora, Nobara, openSUSE, …) — download the `.rpm`
from the [releases page](https://github.com/glengerbush/PEA/releases):

```bash
sudo dnf install ./PEA-*.rpm     # openSUSE: sudo zypper install ./PEA-*.rpm
```

**Anything else** — the AppImage, one command, no root:

```bash
curl -fsSL https://raw.githubusercontent.com/glengerbush/PEA/main/scripts/install-desktop.sh | bash
```

This installs the AppImage into `~/.local/bin` with a launcher entry; it
self-updates in place from then on. The AppImage bundles its own WebKitGTK —
if it aborts with an EGL error or opens a blank window, prefer your distro's
native package above. On NVIDIA proprietary drivers, a blank window can also
be worked around with `WEBKIT_DISABLE_DMABUF_RENDERER=1`.

### macOS

Download the `.dmg` and drag to Applications. The build is
unsigned, so the **first** launch needs System Settings → Privacy & Security →
**Open Anyway** (or `xattr -cr /Applications/PEA.app`). Updates never
re-trigger the prompt.

### From source

```bash
pnpm install && pnpm build                            # types + frontend SPA
FRONTEND_BUILD_DIR=packages/frontend/build \
  cargo run -p pea-engine -- --data-dir ~/.local/share/pea --port 47200
# then open http://127.0.0.1:47200 — or:
# cd apps/desktop && pnpm tauri dev                   # full desktop window
```

## Importing your email

Import once from static files via **Import Archive** in the app. Two formats:

- **Mbox:** one or more `.mbox` files, an Apple Mail `.mbox` package, or a
  folder of them (scanned recursively). Pick the file or folder on your
  machine with the native picker; files are read in place, so this handles
  very large archives without copying them.
- **EML:** a zip archive of `.eml` files; the folder structure inside the
  zip is preserved.

Folder structure is preserved from the mailbox layout and email headers where
possible, and imports of the same mailbox can be merged into one source.

## Searching

Search applies as you type, full-text over subjects, bodies, senders,
recipients, and attachment text (PDF, DOCX, XLSX). Scope the search by field,
filter by source, tag, or attachments, sort by any column; every view is a
URL you can bookmark. Exact duplicates get one-click cleanup and
near-duplicates are surfaced for review under Duplicates.

## Upgrading

The app updates itself: at launch it checks GitHub Releases, and **Update
now** downloads the signed update, verifies it, installs, and restarts. Your
archive is never touched by updates; schema migrations run automatically.
AppImage and macOS installs are fully hands-off; `.deb`/`.rpm` may show a
package-install prompt; `makepkg` installs update by re-running `makepkg -si`.
Downgrading past a schema migration isn't supported, restore the data folder
from a backup instead.

## API Reference

The desktop app serves its API in-process over `pea://`, no network
socket. The same endpoints (prefixed `/api/v1`) are available over local HTTP
when you run the standalone engine
(`pea-engine --data-dir ~/.local/share/pea --port 47200`, 127.0.0.1 only),
handy for scripting against your archive. See the
[API overview](./api/index.md).
