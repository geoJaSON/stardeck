#!/usr/bin/env bash
# Build a .deb package for Stardeck (Linux Mint / Ubuntu / Debian).
#
# Requires: cargo, dpkg-deb (>= 1.19), python3 (stdlib only).
# Output:   Output/stardeck_<version>-1_<arch>.deb
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

VERSION="$(awk -F\" '/^version *=/ {print $2; exit}' Cargo.toml)"
ARCH="$(dpkg --print-architecture)"
PKG="stardeck_${VERSION}-1_${ARCH}"
STAGE="target/deb/${PKG}"

echo ">> cargo build --release"
cargo build --release

echo ">> staging in ${STAGE}"
rm -rf "$STAGE"
install -d "$STAGE/DEBIAN"
install -d "$STAGE/usr/bin"
install -d "$STAGE/usr/share/applications"
install -d "$STAGE/usr/share/doc/stardeck"
install -d "$STAGE/usr/share/pixmaps"
for size in 16 32 48 64; do
    install -d "$STAGE/usr/share/icons/hicolor/${size}x${size}/apps"
done

install -m 755 target/release/stardeck "$STAGE/usr/bin/stardeck"
strip "$STAGE/usr/bin/stardeck"

install -m 644 packaging/stardeck.desktop \
    "$STAGE/usr/share/applications/stardeck.desktop"

echo ">> extracting icons from icon.ico"
ICONS_DIR="target/deb/icons"
rm -rf "$ICONS_DIR"
python3 packaging/extract-ico.py icon.ico "$ICONS_DIR"

for size in 16 32 48 64; do
    src="${ICONS_DIR}/icon-${size}.png"
    if [[ ! -f "$src" ]]; then
        echo "!! missing extracted icon: $src" >&2
        exit 1
    fi
    install -m 644 "$src" \
        "$STAGE/usr/share/icons/hicolor/${size}x${size}/apps/stardeck.png"
done
install -m 644 "${ICONS_DIR}/icon-64.png" \
    "$STAGE/usr/share/pixmaps/stardeck.png"

cat > "$STAGE/usr/share/doc/stardeck/copyright" <<EOF
Stardeck
Upstream-Contact: Jason Jordan <jasjordan@proton.me>
EOF

INSTALLED_KB="$(du -sk "$STAGE/usr" | cut -f1)"
sed -e "s|__VERSION__|${VERSION}|" \
    -e "s|__ARCH__|${ARCH}|" \
    -e "s|__SIZE__|${INSTALLED_KB}|" \
    packaging/control.template > "$STAGE/DEBIAN/control"

echo ">> dpkg-deb --build"
mkdir -p Output
dpkg-deb --build --root-owner-group "$STAGE" "Output/${PKG}.deb"

echo
echo "✓ Built: Output/${PKG}.deb"
echo "  Install: sudo apt install ./Output/${PKG}.deb"
