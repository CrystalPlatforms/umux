#!/bin/sh
# install.sh — install the `umux` CLI from GitHub Releases (issue #65).
#
#   curl -fsSL https://raw.githubusercontent.com/CrystalPlatforms/umux/main/install.sh | sh
#
# Downloads the latest release's asset for this platform, extracts the CLI
# binary and installs it to ~/.local/bin (override with UMUX_INSTALL_DIR).
# Nothing is written outside the install directory and the system's own
# package formats stay untouched.
#
#   macOS : the universal .dmg is mounted and the CLI is copied out of
#           umux.app/Contents/MacOS/umux (same binary the app bundles).
#   Linux : the .deb is opened in place and usr/bin/umux is extracted
#           (needs ar from binutils and tar — present on any distro).
#
# Environment overrides (also the test hooks — see install.test.ts):
#   UMUX_OS           force the platform   (macos | linux)
#   UMUX_ARCH         force the arch       (x86_64 | aarch64)
#   UMUX_VERSION      pin the version      (e.g. 1.0.4) instead of querying
#                     the latest release — makes runs deterministic offline
#   UMUX_INSTALL_DIR  target directory     (default ~/.local/bin)
#
# Flags: --dry-run prints the detected platform, version, chosen asset and
# target without downloading or installing anything.
#
# Zero-cost policy: GitHub Releases is the only source; no signing required.

set -eu

REPO="CrystalPlatforms/umux"
INSTALL_DIR="${UMUX_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf '%s\n' "$*"; }
err() { printf 'umux installer: %s\n' "$*" >&2; exit 1; }

os_detect() {
  if [ -n "${UMUX_OS:-}" ]; then say "$UMUX_OS"; return; fi
  case "$(uname -s)" in
    Darwin) say macos ;;
    Linux) say linux ;;
    *) uname -s ;;
  esac
}

arch_detect() {
  if [ -n "${UMUX_ARCH:-}" ]; then say "$UMUX_ARCH"; return; fi
  case "$(uname -m)" in
    arm64 | aarch64) say aarch64 ;;
    x86_64 | amd64) say x86_64 ;;
    *) uname -m ;;
  esac
}

# Resolve the latest release TAG unless pinned. Parses tag_name out of the
# API JSON with plain sed — no jq dependency.
version_resolve() {
  if [ -n "${UMUX_VERSION:-}" ]; then
    # Normalize a pinned value: assets embed the bare version, URLs take the
    # v-prefixed tag — both are derived below, either input form is fine.
    say "v${UMUX_VERSION#v}"
    return
  fi
  fetch "https://api.github.com/repos/$REPO/releases/latest" |
    sed -n 's/.*"tag_name":[[:space:]]*"\([^"]*\)".*/\1/p' |
    head -n 1
}

fetch() { # fetch <url> — to stdout (API) ; fetch -o <file> <url> for files
  if [ "$1" = "-o" ]; then
    file="$2"; url="$3"
    if command -v curl >/dev/null 2>&1; then
      curl -fsSL -o "$file" "$url"
    elif command -v wget >/dev/null 2>&1; then
      wget -qO "$file" "$url"
    else
      err "need curl or wget to download (none found)"
    fi
  else
    url="$1"
    if command -v curl >/dev/null 2>&1; then
      curl -fsSL "$url"
    elif command -v wget >/dev/null 2>&1; then
      wget -qO- "$url"
    else
      err "need curl or wget to download (none found)"
    fi
  fi
}

# --- Detect platform and pick the asset ---------------------------------

OS=$(os_detect)
ARCH=$(arch_detect)
ASSET=""

case "$OS" in
  macos)
    # One universal image serves both Apple architectures.
    ASSET="umux_%V_universal.dmg"
    ;;
  linux)
    case "$ARCH" in
      x86_64) ASSET="umux_%V_amd64.deb" ;;
      *) err "unsupported platform: linux/$ARCH — release binaries are x86_64 only right now" ;;
    esac
    ;;
  *)
    err "unsupported platform: $OS/$ARCH — grab an installer from https://github.com/$REPO/releases"
    ;;
esac

TAG=$(version_resolve)
[ -n "$TAG" ] || err "could not resolve the latest version (network?)"
# Asset names embed the bare version ("umux_1.0.4_..."), the download URL
# path takes the v-prefixed tag ("/releases/download/v1.0.4/...").
VERSION=${TAG#v}
ASSET_NAME=$(echo "$ASSET" | sed "s/%V/$VERSION/g")
ASSET_URL="https://github.com/$REPO/releases/download/$TAG/$ASSET_NAME"

if [ "${1:-}" = "--dry-run" ]; then
  say "umux installer (dry run)"
  say "  OS:      $OS"
  say "  arch:    $ARCH"
  say "  version: $VERSION${UMUX_VERSION:+ (pinned)}"
  say "  asset:   $ASSET_NAME"
  say "  target:  $INSTALL_DIR/umux"
  exit 0
fi

# --- Download ------------------------------------------------------------

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT INT TERM
PKG="$WORK/$ASSET_NAME"
say "Downloading $ASSET_NAME ..."
fetch -o "$PKG" "$ASSET_URL" || err "download failed: $ASSET_URL"
[ -s "$PKG" ] || err "downloaded file is empty — wrong asset name? ($ASSET_NAME)"

mkdir -p "$INSTALL_DIR"

# --- Extract + install ----------------------------------------------------

case "$OS" in
  macos)
    MNT="$WORK/mnt"
    mkdir -p "$MNT"
    hdiutil attach -nobrowse -quiet -mountpoint "$MNT" "$PKG" ||
      err "could not mount $ASSET_NAME"
    SRC="$MNT/umux.app/Contents/MacOS/umux"
    if [ ! -f "$SRC" ]; then
      hdiutil detach "$MNT" >/dev/null 2>&1 || true
      err "CLI not found inside the image (unexpected bundle layout)"
    fi
    cp "$SRC" "$INSTALL_DIR/umux"
    hdiutil detach "$MNT" >/dev/null 2>&1 || true
    # The downloaded image carries the macOS quarantine flag; a CLI copied
    # out of it would be stopped by Gatekeeper on first run. Drop it.
    xattr -d com.apple.quarantine "$INSTALL_DIR/umux" 2>/dev/null || true
    ;;
  linux)
    command -v ar >/dev/null 2>&1 ||
      err "need 'ar' (binutils) to unpack the .deb — or install the .deb itself instead"
    command -v tar >/dev/null 2>&1 || err "need tar to unpack the .deb"
    MEMBER=""
    for candidate in data.tar.gz data.tar.xz data.tar.zst data.tar; do
      if ar p "$PKG" "$candidate" >/dev/null 2>&1; then MEMBER="$candidate"; break; fi
    done
    [ -n "$MEMBER" ] || err "could not read the .deb payload"
    # The payload paths appear with or without the leading ./ depending on
    # the packaging tool — try both, first hit wins.
    EXTRACTED=""
    for path in usr/bin/umux ./usr/bin/umux; do
      if ar p "$PKG" "$MEMBER" | tar -x -C "$WORK" -O "$path" > "$WORK/umux" 2>/dev/null; then
        EXTRACTED="$WORK/umux"
        break
      fi
    done
    [ -n "$EXTRACTED" ] || err "usr/bin/umux not found inside the .deb"
    mv "$EXTRACTED" "$INSTALL_DIR/umux"
    ;;
esac

chmod +x "$INSTALL_DIR/umux"

# --- Post-install check ---------------------------------------------------
# Only meaningful for a native install — cross-extracting (e.g. the Linux
# binary onto a mac for testing) would fail with "exec format error" and
# say nothing about the extraction itself, which already succeeded.
case "$(uname -s)" in
  Darwin) HOST_OS=macos ;;
  Linux) HOST_OS=linux ;;
  *) HOST_OS=other ;;
esac

if [ "$OS" = "$HOST_OS" ]; then
  if ! "$INSTALL_DIR/umux" --version >/dev/null 2>&1; then
    err "installed binary failed to run — see https://github.com/$REPO#umux-on-your-path"
  fi
fi

if [ "$OS" = "$HOST_OS" ]; then
  say "Installed: $INSTALL_DIR/umux ($("$INSTALL_DIR/umux" --version))"
else
  say "Installed: $INSTALL_DIR/umux (cross-extracted $OS binary — not executed here)"
fi
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    say ""
    say "NOTE: $INSTALL_DIR is not on your PATH. Add it:"
    say "  echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> ~/.zshrc   # or ~/.bashrc"
    ;;
esac
