#!/usr/bin/env bash
# codesign files with a Developer ID identity — the macOS counterpart of
# packaging/windows/sign.ps1.
#
# Signs whatever paths are passed (Mach-O binaries, .app bundles, .dmg):
#   .app  -> codesign --deep --options runtime --timestamp
#   .dmg  -> plain codesign
#   other -> codesign --options runtime --timestamp
#
# Set MACOS_SIGNING_IDENTITY to the certificate's common name, e.g.
#   MACOS_SIGNING_IDENTITY="Developer ID Application: Learning Machines LLC (TEAMID)"
# (or a keychain item SHA-1). Unset -> prints a warning and exits 0, so test
# builds still produce unsigned-but-usable bundles.
#
# Usage: packaging/macos/sign.sh MatCraft.app dist/release/matcraft.dmg ...
set -euo pipefail

warn() {
  if [ "${GITHUB_ACTIONS:-}" = "true" ]; then echo "::warning::$*"; else echo "warning: $*" >&2; fi
}

if [ $# -eq 0 ]; then echo "usage: $0 <file-or-bundle>..." >&2; exit 2; fi

if [ -z "${MACOS_SIGNING_IDENTITY:-}" ]; then
  warn "MACOS_SIGNING_IDENTITY not set: leaving unsigned: $*"
  exit 0
fi

for f in "$@"; do
  [ -e "$f" ] || { echo "sign.sh: no such path: $f" >&2; exit 1; }
  case "$f" in
    *.app) args=(--deep --options runtime) ;;
    *.dmg) args=() ;;
    *)     args=(--options runtime) ;;
  esac
  echo "codesign $f"
  # ${args[@]+...}: bash 3.2 (macOS /bin/bash) treats "${args[@]}" on an empty
  # array as unbound under set -u.
  codesign --force --timestamp ${args[@]+"${args[@]}"} --sign "$MACOS_SIGNING_IDENTITY" "$f"
  codesign --verify --verbose=2 "$f" 2>/dev/null || true
done
