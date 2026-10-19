#!/usr/bin/env bash
# Build and package MatCraft for macOS.
#
# Produces, in $DIST (default: dist/release):
#   matcraft-<version>-macos-<arch>.dmg            MatCraft.app + /Applications link
#   matcraft-<version>-macos-<arch>-portable.tar.gz  matcraft + matcraft-cli, unsigned-adhoc
#
# The app bundle carries both binaries — Contents/MacOS/matcraft (the bundle
# executable) and Contents/MacOS/matcraft-cli — mirroring how the Windows MSI
# and zip ship studio + CLI together. The portable tarball is the counterpart
# of the Windows portable zip.
#
# The .icns is built from the existing PNG set in assets/app-icon (sips
# resamples to the iconset sizes, iconutil packs it) — same original art,
# no new assets.
#
# Signing is OPTIONAL and delegated to sign.sh (skipped with a warning when
# MACOS_SIGNING_IDENTITY is unset), like the Windows track's sign.ps1.
# Notarization likewise runs only when MACOS_NOTARY_PROFILE names a stored
# notarytool keychain profile (`xcrun notarytool store-credentials`).
# An unsigned .app still runs after the user clears the quarantine bit:
#   xattr -d com.apple.quarantine /Applications/MatCraft.app
#
# Needs: macOS with Xcode CLT (iconutil, sips, hdiutil, codesign), Rust.
#
# Usage:
#   packaging/macos/package.sh                       # native arch build
#   packaging/macos/package.sh --arch x86_64         # cross-build on arm64 (and vice versa)
#   packaging/macos/package.sh --arch universal      # lipo both archs into one bundle
#   packaging/macos/package.sh --skip-build --skip-dmg
set -euo pipefail

ARCH="$(uname -m)"   # arm64 or x86_64
SKIP_BUILD=0
SKIP_DMG=0
while [ $# -gt 0 ]; do
  case "$1" in
    --arch) ARCH="${2:?--arch needs x86_64, aarch64/arm64 or universal}"; shift 2 ;;
    --arch=*) ARCH="${1#*=}"; shift ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    --skip-dmg) SKIP_DMG=1; shift ;;
    -h|--help) grep -E '^#( |$)' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

# Normalize: rust targets say aarch64, uname says arm64.
case "$ARCH" in
  arm64|aarch64) ARCH=aarch64;  TARGETS="aarch64-apple-darwin" ;;
  x86_64)        TARGETS="x86_64-apple-darwin" ;;
  universal)     TARGETS="aarch64-apple-darwin x86_64-apple-darwin" ;;
  *) echo "unsupported --arch '$ARCH' (x86_64, aarch64 or universal)" >&2; exit 2 ;;
esac

step() { printf '==> %s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }
warn() {
  if [ "${GITHUB_ACTIONS:-}" = "true" ]; then echo "::warning::$*"; else echo "warning: $*" >&2; fi
}

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
HERE="$(cd "$(dirname "$0")" && pwd)"

# The version lives in one place: [workspace.package] version in the root Cargo.toml.
VERSION="${MATCRAFT_VERSION:-}"
if [ -z "$VERSION" ]; then
  VERSION="$(sed -n '/^\[workspace\.package\]/,/^\[/ s/^[[:space:]]*version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$ROOT/Cargo.toml" | head -n1)"
fi
[ -n "$VERSION" ] || die "could not read [workspace.package] version from Cargo.toml"

DIST="${DIST:-$ROOT/dist/release}"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
ICONS="$ROOT/assets/app-icon"
mkdir -p "$DIST"

export MATCRAFT_BUILD_SHA="${MATCRAFT_BUILD_SHA:-$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || true)}"
export MATCRAFT_BUILD_DATE="${MATCRAFT_BUILD_DATE:-$(date -u +%F)}"

echo "MatCraft $VERSION for macOS $ARCH ($TARGETS)"

if [ "$SKIP_BUILD" -eq 0 ]; then
  for t in $TARGETS; do
    step "cargo build ($t)"
    cargo build --release --locked -p matcraft -p matcraft-cli --target "$t"
  done
fi

STAGE="$TARGET_DIR/macos-package/$ARCH"
rm -rf "$STAGE"
mkdir -p "$STAGE"

# Stage one binary per name into $STAGE/bin: native arch copies it, universal
# lipo's the per-arch builds together.
stage_bin() {
  local name="$1" out="$STAGE/bin/$1" src
  if [ "$ARCH" = universal ]; then
    local inputs=()
    for t in $TARGETS; do
      src="$TARGET_DIR/$t/release/$name"
      [ -f "$src" ] || die "$src not found"
      inputs+=("$src")
    done
    lipo -create "${inputs[@]}" -output "$out"
  else
    src="$TARGET_DIR/$TARGETS/release/$name"
    [ -f "$src" ] || die "$src not found (run without --skip-build first)"
    cp "$src" "$out"
  fi
  chmod +x "$out"   # lipo output is 644
}

mkdir -p "$STAGE/bin"
stage_bin matcraft
stage_bin matcraft-cli

# ---- .icns from the existing PNGs -------------------------------------------
# iconset sizes: 16/32/128/256/512/1024 (+@2x). We have 16/32/64/256 source art;
# sips resamples up to the rest (the icon is flat procedural art — upsampling
# is clean).
step "matcraft.icns"
ICONSET="$STAGE/matcraft.iconset"
mkdir -p "$ICONSET"
gen_icon() { # $1 = pixel size, $2 = iconset filename
  sips -z "$1" "$1" "$ICONS/matcraft-256.png" --out "$ICONSET/$2" >/dev/null
}
for size in 16 32 128 256 512; do
  gen_icon "$size" "icon_${size}x${size}.png"
  gen_icon "$((size * 2))" "icon_${size}x${size}@2x.png"
done
iconutil -c icns "$ICONSET" -o "$STAGE/matcraft.icns"

# ---- MatCraft.app -------------------------------------------------------------
APP="$STAGE/MatCraft.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$STAGE/bin/matcraft" "$STAGE/bin/matcraft-cli" "$APP/Contents/MacOS/"
cp "$STAGE/matcraft.icns" "$APP/Contents/Resources/"
sed "s/__VERSION__/$VERSION/g" "$HERE/Info.plist" > "$APP/Contents/Info.plist"

step "sign (optional)"
"$HERE/sign.sh" "$APP" || die "sign.sh failed"

# ---- portable tarball (Windows portable-zip counterpart) ----------------------
PORTABLE="$STAGE/matcraft-$VERSION-macos-$ARCH-portable"
mkdir -p "$PORTABLE/bin"
cp "$STAGE/bin/matcraft" "$STAGE/bin/matcraft-cli" "$PORTABLE/bin/"
"$HERE/sign.sh" "$PORTABLE/bin/matcraft" "$PORTABLE/bin/matcraft-cli" || die "sign.sh failed"
for f in README.md LICENSE-MIT LICENSE-APACHE NOTICE; do
  if [ -f "$ROOT/$f" ]; then cp "$ROOT/$f" "$PORTABLE/"; fi
done
TARBALL="$DIST/matcraft-$VERSION-macos-$ARCH-portable.tar.gz"
rm -f "$TARBALL"
step "tarball -> $TARBALL"
tar -C "$STAGE" -czf "$TARBALL" "matcraft-$VERSION-macos-$ARCH-portable"

# ---- .dmg ---------------------------------------------------------------------
DMG="$DIST/matcraft-$VERSION-macos-$ARCH.dmg"
if [ "$SKIP_DMG" -eq 0 ]; then
  DMG_STG="$STAGE/dmg"
  mkdir -p "$DMG_STG"
  cp -R "$APP" "$DMG_STG/"
  ln -sfn /Applications "$DMG_STG/Applications"
  rm -f "$DMG"
  step "hdiutil -> $DMG"
  hdiutil create -volname "MatCraft $VERSION" -srcfolder "$DMG_STG" -ov -format UDZO "$DMG"
  "$HERE/sign.sh" "$DMG" || die "sign.sh failed"
  if [ -n "${MACOS_NOTARY_PROFILE:-}" ]; then
    step "notarize ($MACOS_NOTARY_PROFILE)"
    xcrun notarytool submit "$DMG" --keychain-profile "$MACOS_NOTARY_PROFILE" --wait
    xcrun stapler staple "$DMG"
  else
    warn "MACOS_NOTARY_PROFILE not set: skipping notarization (unsigned apps need xattr -d com.apple.quarantine)"
  fi
fi

# Smoke-test the CLI when this machine can run it (mirrors the Windows track).
HOST_ARCH="$(uname -m)"; HOST_ARCH="${HOST_ARCH/arm64/aarch64}"
if [ "$ARCH" = "$HOST_ARCH" ] || [ "$ARCH" = universal ]; then
  step "matcraft-cli --version"
  "$STAGE/bin/matcraft-cli" --version
else
  echo "skipping matcraft-cli --version: an $ARCH build doesn't run on this $HOST_ARCH machine"
fi

step "artifacts"
ls -lh "$TARBALL"
if [ "$SKIP_DMG" -eq 0 ]; then ls -lh "$DMG"; fi
