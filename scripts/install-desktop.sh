#!/usr/bin/env bash
#
# One-command Linux install of the PEA (Personal Email Archive) desktop app.
#
#   curl -fsSL https://raw.githubusercontent.com/glengerbush/PEA/main/scripts/install-desktop.sh | bash
#
# Downloads the latest AppImage from GitHub Releases into ~/.local/bin and adds
# a desktop entry + icon, so it shows up in your launcher like a normal app.
# No root required; works on Arch, Fedora, Debian/Ubuntu, and everything else.
# The app self-updates in place from then on (one click, in-app).
#
set -euo pipefail

REPO="glengerbush/PEA"
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
curl -fL --progress-bar "$asset_url" -o "${BIN_DIR}/pea.AppImage"
chmod +x "${BIN_DIR}/pea.AppImage"

echo "==> Installing icon and desktop entry..."
curl -fsSL "https://raw.githubusercontent.com/${REPO}/main/apps/desktop/src-tauri/icons/512x512.png" \
	-o "${ICON_DIR}/pea.png" || true

cat > "${APP_DIR}/pea.desktop" <<EOF
[Desktop Entry]
Name=PEA
Comment=Personal email archiving
Exec=${BIN_DIR}/pea.AppImage
Icon=pea
Terminal=false
Type=Application
Categories=Utility;Office;
EOF
update-desktop-database "$APP_DIR" 2>/dev/null || true

echo
echo "Done. Launch \"PEA\" from your app menu, or run:"
echo "  ${BIN_DIR}/pea.AppImage"
echo "Your archive lives in \${XDG_DATA_HOME:-~/.local/share}/pea."
