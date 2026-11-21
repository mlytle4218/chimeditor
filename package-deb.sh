#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

VERSION="0.1.0"
ARCH="$(dpkg --print-architecture 2>/dev/null || echo "amd64")"
PKG_NAME="chimeditor"
STAGE_DIR="target/debian/${PKG_NAME}_${VERSION}_${ARCH}"
DEB_FILE="${PKG_NAME}_${VERSION}_${ARCH}.deb"

echo "=== Building ChimEditor Release Binary ==="
cargo build --release

echo "=== Staging Debian Package Files in ${STAGE_DIR} ==="
rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR/DEBIAN"
mkdir -p "$STAGE_DIR/usr/bin"
mkdir -p "$STAGE_DIR/usr/share/man/man1"
mkdir -p "$STAGE_DIR/usr/share/chimeditor"
mkdir -p "$STAGE_DIR/usr/share/doc/chimeditor"

# 1. Install binaries
install -m 755 target/release/chimeditor "$STAGE_DIR/usr/bin/chimeditor"
ln -sf chimeditor "$STAGE_DIR/usr/bin/shimeditor"

# 2. Install man pages
gzip -9 -c chimeditor.1 > "$STAGE_DIR/usr/share/man/man1/chimeditor.1.gz"
ln -sf chimeditor.1.gz "$STAGE_DIR/usr/share/man/man1/shimeditor.1.gz"

# 3. Install default configuration template
install -m 644 config.yaml "$STAGE_DIR/usr/share/chimeditor/config.yaml"

# 4. Install documentation and copyright
cat << 'EOF' > "$STAGE_DIR/usr/share/doc/chimeditor/copyright"
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: chimeditor
Source: https://github.com/chimeditor/chimeditor

Files: *
Copyright: 2026 ChimEditor Authors
License: MIT
EOF

cat << EOF > "$STAGE_DIR/usr/share/doc/chimeditor/changelog.Debian"
chimeditor (${VERSION}) stable; urgency=medium

  * Initial Debian package release.
  * Stores user config and logs in ~/.config/shimeditor.

 -- ChimEditor Developers <support@chimeditor.org>  $(date -R)
EOF
gzip -9 "$STAGE_DIR/usr/share/doc/chimeditor/changelog.Debian"

# 5. Create DEBIAN/control
cat << EOF > "$STAGE_DIR/DEBIAN/control"
Package: ${PKG_NAME}
Version: ${VERSION}
Section: video
Priority: optional
Architecture: ${ARCH}
Depends: libc6, libasound2t64 | libasound2, ffmpeg, zip, unzip
Maintainer: ChimEditor Developers <support@chimeditor.org>
Description: Terminal-driven audio/video skip editor
 ChimEditor is a fast, keyboard-driven media editor designed for marking,
 auditing, and trimming skip sections in audio and video files. Skips are
 packaged within .cme project files, audited in real-time with synthesizing
 chimes, and rendered losslessly into non-skipped media using FFmpeg.
 User configuration and logs are maintained in ~/.config/shimeditor.
EOF

# Ensure standard permissions required by dpkg-deb
find "$STAGE_DIR" -type d -exec chmod 755 {} +
find "$STAGE_DIR" -type f -exec chmod 644 {} +
chmod 755 "$STAGE_DIR/usr/bin/chimeditor"
chmod 755 "$STAGE_DIR/DEBIAN"
chmod 644 "$STAGE_DIR/DEBIAN/control"

# 6. Build the Debian installer (.deb package)
echo "=== Generating Debian Package: ${DEB_FILE} ==="
dpkg-deb --build --root-owner-group "$STAGE_DIR" "$DEB_FILE"

echo "=== Package Built Successfully ==="
dpkg-deb --info "$DEB_FILE"
dpkg-deb --contents "$DEB_FILE"
