#!/usr/bin/env bash
#
# One-command Linux install of the Open Archiver desktop app.
#
#   curl -fsSL https://raw.githubusercontent.com/glengerbush/OpenArchiver/main/scripts/install-desktop.sh | bash
#
# Downloads the latest AppImage from GitHub Releases into ~/.local/bin and adds
# a desktop entry + icon, so it shows up in your launcher like a normal app.
# No root required; works on Arch, Fedora, Debian/Ubuntu, and everything else.
# The app self-updates in place from then on (one click, in-app).
#
set -euo pipefail

REPO="glengerbush/OpenArchiver"
BIN_DIR="${HOME}/.local/bin"
APP_DIR="${HOME}/.local/share/applications"
ICON_DIR="${HOME}/.local/share/icons/hicolor/512x512/apps"

arch="$(uname -m)"
case "$arch" in
	x86_64) suffix="amd64" ;;
	aarch64) suffix="aarch64" ;;
	*) echo "Unsupported architecture: $arch"; exit 1 ;;
esac

echo "==> Finding the latest release..."
asset_url="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" |
	grep -o "https://[^\"]*${suffix}[^\"]*\.AppImage" | head -1)"
if [ -z "$asset_url" ]; then
	echo "No AppImage asset found on the latest release of ${REPO}." >&2
	exit 1
fi

mkdir -p "$BIN_DIR" "$APP_DIR" "$ICON_DIR"
echo "==> Downloading $(basename "$asset_url")..."
curl -fL --progress-bar "$asset_url" -o "${BIN_DIR}/open-archiver.AppImage"
chmod +x "${BIN_DIR}/open-archiver.AppImage"

echo "==> Installing icon and desktop entry..."
curl -fsSL "https://raw.githubusercontent.com/${REPO}/main/apps/desktop/src-tauri/icons/512x512.png" \
	-o "${ICON_DIR}/open-archiver.png" || true

cat > "${APP_DIR}/open-archiver.desktop" <<EOF
[Desktop Entry]
Name=Open Archiver
Comment=Personal email archiving
Exec=${BIN_DIR}/open-archiver.AppImage
Icon=open-archiver
Terminal=false
Type=Application
Categories=Utility;Office;
EOF
update-desktop-database "$APP_DIR" 2>/dev/null || true

echo
echo "Done. Launch \"Open Archiver\" from your app menu, or run:"
echo "  ${BIN_DIR}/open-archiver.AppImage"
echo "Your archive lives in \${XDG_DATA_HOME:-~/.local/share}/open-archiver."
