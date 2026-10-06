#!/bin/sh
# Rasmalai installer: curl -fsSL https://rasmalai.rovelstars.com/install.sh | sh
# POSIX sh. Needs curl or wget, plus tar.
# Env knobs: RNX_VERSION (default: latest), RNX_PREFIX (default: see below),
#            DRY_RUN=1 (detect and print only, no download/install).
#
# Install locations follow the platform base-directory convention:
#   Linux/macOS: <root>/bin + <root>/lib, where root defaults to the parent
#     of $XDG_BIN_HOME when set, else $HOME/.local (the XDG default user
#     prefix, so `~/.local/bin/rnx` lands on PATH-adjacent ground).
#   Windows (Git Bash/MSYS/Cygwin): root defaults to %LOCALAPPDATA%\rnx.
# Pass --prefix DIR (or set RNX_PREFIX) to override the root.
set -u

REPO="rovelstars/rasmalai"
VERSION="${RNX_VERSION:-latest}"
PREFIX="${RNX_PREFIX:-}"

while [ $# -gt 0 ]; do
  case "$1" in
    --prefix)
      PREFIX="$2"
      shift 2
      ;;
    --prefix=*)
      PREFIX="${1#--prefix=}"
      shift
      ;;
    -h|--help)
      echo "usage: install.sh [--prefix DIR]  (RNX_VERSION pins a release)"
      exit 0
      ;;
    *)
      echo "usage: install.sh [--prefix DIR]" >&2
      exit 1
      ;;
  esac
done

if [ -z "$PREFIX" ]; then
  if [ -n "${XDG_BIN_HOME:-}" ]; then
    PREFIX="$(dirname "$XDG_BIN_HOME")"
  elif [ -n "${LOCALAPPDATA:-}" ] && command -v cygpath >/dev/null 2>&1; then
    case "$(uname -s)" in
      MINGW*|MSYS*|CYGWIN*|Windows_NT)
        PREFIX="$(cygpath -u "$LOCALAPPDATA/rnx")"
        ;;
      *)
        PREFIX="$HOME/.local"
        ;;
    esac
  else
    PREFIX="$HOME/.local"
  fi
fi
BIN_DIR="$PREFIX/bin"
LIB_DIR="$PREFIX/lib"

fail() {
  echo "error: $1" >&2
  exit 1
}

detect_target() {
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$os" in
    Linux) os="linux" ;;
    Darwin) os="macos" ;;
    MINGW*|MSYS*|CYGWIN*|Windows_NT) os="windows" ;;
    *) fail "unsupported OS: $os" ;;
  esac
  case "$arch" in
    x86_64|amd64) arch="x86_64" ;;
    arm64|aarch64) arch="aarch64" ;;
    *) fail "unsupported architecture: $arch (need x86_64 or aarch64)" ;;
  esac
  case "$os-$arch" in
    linux-x86_64) printf "x86_64-linux" ;;
    linux-aarch64) printf "aarch64-linux" ;;
    macos-aarch64) printf "aarch64-macos" ;;
    windows-x86_64) printf "x86_64-windows" ;;
    *) fail "no release build for $os-$arch (macOS ships Apple Silicon only)" ;;
  esac
}

TARGET="$(detect_target)"

if [ "$VERSION" = "latest" ]; then
  BASE="https://github.com/$REPO/releases/latest/download"
else
  BASE="https://github.com/$REPO/releases/download/$VERSION"
fi
ARCHIVE="rnx-$TARGET.tar.gz"
URL="$BASE/$ARCHIVE"

if [ "${DRY_RUN:-0}" = "1" ]; then
  echo "target=$TARGET"
  echo "url=$URL"
  echo "bin=$BIN_DIR"
  echo "lib=$LIB_DIR"
  exit 0
fi

if command -v curl >/dev/null 2>&1; then
  fetch() { curl -fsSL --retry 3 "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then
  fetch() { wget -q --tries=3 -O "$2" "$1"; }
else
  fail "need curl or wget to download $URL"
fi

command -v tar >/dev/null 2>&1 || fail "need tar to unpack the archive"

TMP="$(mktemp -d 2>/dev/null || mktemp -d -t rnx)" || fail "cannot create temp dir"
trap 'rm -rf "$TMP"' EXIT INT TERM

echo "downloading $URL"
fetch "$URL" "$TMP/$ARCHIVE" || fail "download failed: $URL"
tar -xzf "$TMP/$ARCHIVE" -C "$TMP" || fail "unpack failed"
STAGE="$TMP/rnx-$TARGET"
[ -d "$STAGE/bin" ] || fail "archive has no bin/ directory"

mkdir -p "$BIN_DIR" "$LIB_DIR" || fail "cannot create $BIN_DIR"
cp -r "$STAGE/bin/." "$BIN_DIR/" || fail "cannot install into $BIN_DIR"
if [ -d "$STAGE/lib" ]; then
  cp -r "$STAGE/lib/." "$LIB_DIR/" || fail "cannot install into $LIB_DIR"
fi
if [ -f "$BIN_DIR/rnx" ]; then
  chmod +x "$BIN_DIR/rnx"
fi

if [ -x "$BIN_DIR/rnx" ]; then
  BIN="$BIN_DIR/rnx"
elif [ -x "$BIN_DIR/rnx.exe" ]; then
  BIN="$BIN_DIR/rnx.exe"
else
  fail "installed but no runnable binary found in $BIN_DIR"
fi
INSTALLED_VERSION="$("$BIN" --version 2>/dev/null | cut -d' ' -f2)"
[ -z "$INSTALLED_VERSION" ] && INSTALLED_VERSION="$VERSION"

if "$BIN" fetch-std >/dev/null 2>&1; then
  echo "stdlib cache: seeded"
else
  echo "warning: stdlib cache seeding failed (offline install? run \`rnx fetch-std\` later)" >&2
fi

PROFILE_HINT=""
case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *)
    SHELL_NAME="$(basename "${SHELL:-sh}")"
    case "$SHELL_NAME" in
      fish)
        CONFIG="$HOME/.config/fish/config.fish"
        mkdir -p "$(dirname "$CONFIG")"
        echo "fish_add_path \"$BIN_DIR\"" >> "$CONFIG"
        PROFILE_HINT="$CONFIG"
        ;;
      zsh)
        CONFIG="${ZDOTDIR:-$HOME}/.zshrc"
        echo "export PATH=\"$BIN_DIR:\$PATH\"" >> "$CONFIG"
        PROFILE_HINT="$CONFIG"
        ;;
      *)
        CONFIG="$HOME/.bashrc"
        echo "export PATH=\"$BIN_DIR:\$PATH\"" >> "$CONFIG"
        PROFILE_HINT="$CONFIG"
        ;;
    esac
    ;;
esac

echo "rasmalai installed successfully"
echo "  binary:   $BIN"
echo "  version:  $INSTALLED_VERSION"
if [ -n "$PROFILE_HINT" ]; then
  echo "restart your shell or run: source $PROFILE_HINT"
else
  echo "$BIN_DIR is already on your PATH"
fi
