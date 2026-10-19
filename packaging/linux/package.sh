#!/usr/bin/env bash
# Build and package MatCraft for Linux.
#
# Produces, in $DIST (default: dist/release):
#   matcraft-<version>-linux-<arch>.tar.gz   portable bundle (bin/ + share/)
#   matcraft_<version>_<debarch>.deb         Debian package (dpkg-deb)
#
# The portable tarball mirrors the Windows portable zip: both binaries plus
# desktop-integration files laid out in the standard bin/ + share/ tree, so
# `sudo cp -r bin share /usr/` (or $HOME/.local/) gives the same install as the
# .deb.
#
# Runtime dependencies are minimal on purpose: winit talks X11 through x11rb
# (pure Rust, a raw socket — no libX11/libxcb) and dlopen()s libwayland-client
# and libxkbcommon; wgpu dlopen()s libvulkan/libEGL. The .deb therefore hard-
# Depends only on libc6 and libgcc-s1 and puts the display/GPU libraries in
# Recommends — apt installs them by default, and on a headless /
# --no-install-recommends box the CLI still works. Verify with `ldd` on the
# built binary if in doubt.
#
# No AppImage: it would need linuxdeploy/appimagetool downloads, a FUSE runtime
# and a careful oldest-glibc build floor. The tarball already covers the
# "run anywhere without installing" case and the .deb covers managed systems.
#
# Needs: Rust toolchain (+ the target for cross builds), tar, and dpkg-deb
# (skip the .deb with --tarball-only). aarch64 cross builds also need a linker,
# e.g. `apt install gcc-aarch64-linux-gnu` and
# CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc.
#
# Usage:
#   packaging/linux/package.sh                       # native x86_64 build
#   packaging/linux/package.sh --arch aarch64        # needs the aarch64 target + linker
#   packaging/linux/package.sh --skip-build --tarball-only
set -euo pipefail

ARCH=x86_64
SKIP_BUILD=0
SKIP_DEB=0
while [ $# -gt 0 ]; do
  case "$1" in
    --arch) ARCH="${2:?--arch needs x86_64 or aarch64}"; shift 2 ;;
    --arch=*) ARCH="${1#*=}"; shift ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    --tarball-only) SKIP_DEB=1; shift ;;
    -h|--help) grep -E '^#( |$)' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

case "$ARCH" in
  x86_64)  TARGET=x86_64-unknown-linux-gnu;  DEBARCH=amd64 ;;
  aarch64) TARGET=aarch64-unknown-linux-gnu; DEBARCH=arm64 ;;
  *) echo "unsupported --arch '$ARCH' (x86_64 or aarch64)" >&2; exit 2 ;;
esac

step() { printf '==> %s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
HERE="$(cd "$(dirname "$0")" && pwd)"

# The version lives in one place: [workspace.package] version in the root Cargo.toml.
VERSION="${MATCRAFT_VERSION:-}"
if [ -z "$VERSION" ]; then
  VERSION="$(sed -n '/^\[workspace\.package\]/,/^\[/ s/^[[:space:]]*version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$ROOT/Cargo.toml" | head -n1)"
fi
[ -n "$VERSION" ] || die "could not read [workspace.package] version from Cargo.toml"
DEBVERSION="$(printf %s "$VERSION" | tr '-' '~')"   # 0.2.0-rc.1 -> 0.2.0~rc.1 (deb ordering)

DIST="${DIST:-$ROOT/dist/release}"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
BIN="$TARGET_DIR/$TARGET/release"
ICONS="$ROOT/assets/app-icon"
mkdir -p "$DIST"

export MATCRAFT_BUILD_SHA="${MATCRAFT_BUILD_SHA:-$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || true)}"
export MATCRAFT_BUILD_DATE="${MATCRAFT_BUILD_DATE:-$(date -u +%F)}"

echo "MatCraft $VERSION for Linux $ARCH ($TARGET)"

if [ "$SKIP_BUILD" -eq 0 ]; then
  step "cargo build ($TARGET)"
  cargo build --release --locked -p matcraft -p matcraft-cli --target "$TARGET"
fi

[ -f "$BIN/matcraft" ] || die "$BIN/matcraft not found (run without --skip-build first)"
[ -f "$BIN/matcraft-cli" ] || die "$BIN/matcraft-cli not found"

STAGE="$TARGET_DIR/linux-package/$ARCH"
rm -rf "$STAGE"
mkdir -p "$STAGE"

# Lay out the desktop-integration payload shared by the tarball and the .deb
# under $1: bin/ share/applications/ share/icons/hicolor/ share/mime/packages/
# plus the licence/readme files. Binaries are copied separately.
payload() {
  local dest="$1" size f
  mkdir -p "$dest/bin" "$dest/share/applications" "$dest/share/mime/packages" "$dest/share/doc/matcraft"
  cp "$HERE/matcraft.desktop" "$dest/share/applications/matcraft.desktop"
  cp "$HERE/matcraft.xml" "$dest/share/mime/packages/matcraft.xml"
  cp "$HERE/copyright" "$dest/share/doc/matcraft/copyright"
  for size in 16 32 64 256; do
    mkdir -p "$dest/share/icons/hicolor/${size}x${size}/apps"
    cp "$ICONS/matcraft-$size.png" "$dest/share/icons/hicolor/${size}x${size}/apps/matcraft.png"
  done
  for f in README.md LICENSE-MIT LICENSE-APACHE NOTICE; do
    if [ -f "$ROOT/$f" ]; then cp "$ROOT/$f" "$dest/"; fi
  done
}

# ---- portable tarball -------------------------------------------------------
BUNDLE="matcraft-$VERSION-linux-$ARCH"
PORTABLE="$STAGE/$BUNDLE"
payload "$PORTABLE"
cp "$BIN/matcraft" "$BIN/matcraft-cli" "$PORTABLE/bin/"
TARBALL="$DIST/$BUNDLE.tar.gz"
rm -f "$TARBALL"
step "tarball -> $TARBALL"
tar -C "$STAGE" -czf "$TARBALL" "$BUNDLE"

# ---- .deb -------------------------------------------------------------------
DEB="$DIST/matcraft_${DEBVERSION}_$DEBARCH.deb"
if [ "$SKIP_DEB" -eq 0 ]; then
  command -v dpkg-deb >/dev/null 2>&1 || die "dpkg-deb not found (apt install dpkg-dev, or use --tarball-only)"
  DROOT="$STAGE/deb"
  payload "$DROOT/usr"
  # payload() also fills usr/bin and drops readme/licences at usr/ for the
  # tarball layout; the .deb wants binaries in usr/bin and licences already
  # live in usr/share/doc/matcraft/copyright.
  rm -rf "$DROOT/usr/bin" "$DROOT/usr/README.md" "$DROOT/usr/LICENSE-MIT" "$DROOT/usr/LICENSE-APACHE" "$DROOT/usr/NOTICE"
  mkdir -p "$DROOT/usr/bin" "$DROOT/DEBIAN"
  cp "$BIN/matcraft" "$BIN/matcraft-cli" "$DROOT/usr/bin/"

  cat > "$DROOT/DEBIAN/control" <<EOF
Package: matcraft
Version: $DEBVERSION
Architecture: $DEBARCH
Maintainer: ArtCraft Team <https://getartcraft.com>
Installed-Size: $(du -sk "$DROOT/usr" | cut -f1)
Section: science
Priority: optional
Depends: libc6, libgcc-s1
Recommends: libegl1 | libvulkan1, libwayland-client0, libxkbcommon0, libxkbcommon-x11-0
Homepage: https://github.com/storytold/matcraft
Description: MatCraft numerical computing environment
 A clean-room, Rust-native numerical computing environment with an M-code
 compatible language: command window, workspace, editor and figure dock
 (matcraft), plus a command-line interpreter and MCP server (matcraft-cli).
EOF

  # Refresh the mime/desktop/icon caches when the tools exist (they do on any
  # desktop system); harmless when they don't.
  for hook in postinst postrm; do
    cat > "$DROOT/DEBIAN/$hook" <<'EOF'
#!/bin/sh
set -e
command -v update-mime-database >/dev/null 2>&1 && update-mime-database /usr/share/mime >/dev/null || true
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database /usr/share/applications >/dev/null || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor || true
exit 0
EOF
    chmod 755 "$DROOT/DEBIAN/$hook"
  done

  step "dpkg-deb -> $DEB"
  rm -f "$DEB"
  dpkg-deb --root-owner-group --build "$DROOT" "$DEB"
fi

# Smoke-test the CLI when this machine can run it (mirrors the Windows track).
HOST_ARCH="$(uname -m)"
if [ "$ARCH" = "$HOST_ARCH" ]; then
  step "matcraft-cli --version"
  "$BIN/matcraft-cli" --version
else
  echo "skipping matcraft-cli --version: an $ARCH build doesn't run on this $HOST_ARCH machine"
fi

step "artifacts"
ls -lh "$TARBALL"
if [ "$SKIP_DEB" -eq 0 ]; then ls -lh "$DEB"; fi
