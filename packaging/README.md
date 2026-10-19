# MatCraft packaging

One script per OS, each run on its native host. All of them release-build
`matcraft` (Studio GUI) + `matcraft-cli`, read the version from
`[workspace.package]` in the root `Cargo.toml` (override with
`MATCRAFT_VERSION`), honour `DIST` (default `dist/release`) and
`CARGO_TARGET_DIR`, and smoke-test `matcraft-cli --version` when the built
arch can run on the host.

| OS      | Script                       | Artifacts (in `dist/release/`)                                  |
| ------- | ---------------------------- | --------------------------------------------------------------- |
| Windows | `windows/package.ps1`        | `matcraft-<ver>-windows-<arch>.msi`, `…-portable.zip`            |
| Linux   | `linux/package.sh`           | `matcraft-<ver>-linux-<arch>.tar.gz`, `matcraft_<ver>_<arch>.deb` |
| macOS   | `macos/package.sh`           | `matcraft-<ver>-macos-<arch>.dmg`, `…-portable.tar.gz`           |

Naming: `<arch>` is `x64|x86|arm64` on Windows, `x86_64|aarch64` in the
Linux/macOS file names, `amd64|arm64` in the `.deb` file name.

## Windows

```powershell
pwsh packaging\windows\package.ps1 -Arch x64
```

Prereqs: MSVC Rust toolchain + target, Windows SDK, WiX v5 +
`WixToolset.UI.wixext` (see the header comment in `package.ps1`). Signing via
`sign.ps1` when `WINDOWS_CERTIFICATE`/`WINDOWS_CERTIFICATE_PASSWORD` or the
Azure Trusted Signing `AZURE_*` vars are set — otherwise it warns and ships
unsigned.

## Linux

```bash
packaging/linux/package.sh                  # x86_64
packaging/linux/package.sh --arch aarch64   # needs the target + aarch64-linux-gnu-gcc
```

Prereqs: Rust (+ target for cross builds), `tar`, `dpkg-deb` (`dpkg-dev`;
`--tarball-only` skips the .deb). The .deb installs to `/usr/bin` with a
`.desktop` entry, hicolor icons and a `text/x-matlab` mime registration
(`*.m` → "Open with MatCraft", same idea as the MSI's `OpenWithProgids`).
Dependencies stay minimal (`libc6`, `libgcc-s1`): winit uses `x11rb` (no
libX11/libxcb) and dlopen's Wayland/XKB libs; wgpu dlopen's Vulkan/EGL — so
those sit in `Recommends`. The tarball is `bin/` + `share/` — unpack and
`cp -r` into `/usr` or `~/.local` for the same desktop integration.

No AppImage: it needs linuxdeploy/appimagetool, a FUSE runtime and an
oldest-glibc build floor; tarball + .deb cover the same ground with zero extra
tooling.

## macOS

```bash
packaging/macos/package.sh                      # native arch
packaging/macos/package.sh --arch universal     # lipo'd single bundle
```

Prereqs: Xcode CLT (`iconutil`, `sips`, `hdiutil`, `codesign`), Rust.
Builds `MatCraft.app` (both binaries in `Contents/MacOS`, `.icns` generated
from `assets/app-icon/*.png`, `Info.plist` from `macos/Info.plist` with the
version substituted) and wraps it in a UDZO `.dmg` with an `/Applications`
link. The portable tarball carries the two bare binaries.

Signing is opt-in, like `sign.ps1`:

- `MACOS_SIGNING_IDENTITY="Developer ID Application: …"` → `sign.sh`
  codesigns the binaries, `.app` (`--deep --options runtime`) and `.dmg`.
- `MACOS_NOTARY_PROFILE=<keychain profile>` (from `xcrun notarytool
  store-credentials`) → the `.dmg` is submitted to notarytool and stapled.

Unsigned artifacts still work: users clear the quarantine bit once with
`xattr -d com.apple.quarantine /Applications/MatCraft.app`.

## Status

`windows/` produces shipped artifacts today. `linux/` and `macos/` are
written to the same conventions but **authored on Windows and not yet run on
their native hosts** — they pass `bash -n`/review, but the first real run on
Linux/macOS may need small fixes (exact `ldd` output, iconutil/sips edge
cases, hdiutil flags). `.github/workflows/release.yml` exercises them on
GitHub runners, which doubles as the first end-to-end test.
