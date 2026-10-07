#!/bin/sh
# Install rnx from GitHub releases.
#
# Usage:
#   sh install.sh [version] [--prefix DIR]
#
# Install locations follow the platform base-directory convention:
#   Linux/macOS: <prefix>/bin + <prefix>/lib, where prefix defaults to the
#     parent of $XDG_BIN_HOME when set, else $HOME/.local (the XDG default).
#   Windows (Git Bash/MSYS/Cygwin): prefix defaults to %LOCALAPPDATA%\rnx.
# Override with --prefix DIR.
#
# Needs: curl or wget, tar, a C linker (cc) for `rnx build` AOT output.
set -eu

REPO="rovelstars/rasmalai"
VERSION=""
PREFIX=""

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
      sed -n '2,14p' "$0"
      exit 0
      ;;
    *)
      VERSION="$1"
      shift
      ;;
  esac
done

have() { command -v "$1" >/dev/null 2>&1; }

if [ -z "$PREFIX" ]; then
  if [ -n "${XDG_BIN_HOME:-}" ]; then
    PREFIX="$(dirname "$XDG_BIN_HOME")"
  elif [ -n "${LOCALAPPDATA:-}" ] && have cygpath; then
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

if [ -z "$VERSION" ]; then
  if have curl; then
    VERSION="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -1)"
  elif have wget; then
    VERSION="$(wget -qO- "https://api.github.com/repos/$REPO/releases/latest" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -1)"
  else
    echo "install.sh: need curl or wget" >&2
    exit 1
  fi
fi

if [ -z "$VERSION" ]; then
  echo "install.sh: could not resolve a release version" >&2
  exit 1
fi

os="$(uname -s)"
arch="$(uname -m)"
case "$os-$arch" in
  Linux-x86_64) target="x86_64-linux" ;;
  Linux-aarch64|Linux-arm64) target="aarch64-linux" ;;
  Darwin-arm64|Darwin-aarch64) target="aarch64-macos" ;;
  MINGW64_NT-*|MSYS_NT-*|CYGWIN_NT-*|Windows_NT-*) target="x86_64-windows" ;;
  *)
    echo "install.sh: unsupported platform $os-$arch" >&2
    exit 1
    ;;
esac

url="https://github.com/$REPO/releases/download/$VERSION/rnx-$target.tar.gz"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "install.sh: fetching $url"
if have curl; then
  curl -fsSL --retry 3 -o "$tmp/rnx.tar.gz" "$url"
else
  wget -q --tries=3 -O "$tmp/rnx.tar.gz" "$url"
fi

tar xzf "$tmp/rnx.tar.gz" -C "$tmp"
mkdir -p "$PREFIX/bin" "$PREFIX/lib"
cp -r "$tmp/rnx-$target/bin/." "$PREFIX/bin/"
cp -r "$tmp/rnx-$target/lib/." "$PREFIX/lib/" 2>/dev/null || true
chmod +x "$PREFIX/bin/rnx" 2>/dev/null || true

if [ -x "$PREFIX/bin/rnx" ]; then
  BIN="$PREFIX/bin/rnx"
elif [ -x "$PREFIX/bin/rnx.exe" ]; then
  BIN="$PREFIX/bin/rnx.exe"
else
  echo "install.sh: installed but no runnable binary found in $PREFIX/bin" >&2
  exit 1
fi

if ! "$BIN" --version; then
  echo "install.sh: installed binary failed to run" >&2
  exit 1
fi

if "$BIN" fetch-std; then
  echo "install.sh: seeded the standard library cache"
else
  echo "install.sh: warning: stdlib cache seeding failed (offline?); run \`rnx fetch-std\` once online, or \`rnx doctor --repair-std\` to repair it" >&2
fi

case ":$PATH:" in
  *":$PREFIX/bin:"*) ;;
  *) echo "install.sh: add $PREFIX/bin to PATH" ;;
esac
echo "install.sh: rnx $VERSION installed to $PREFIX/bin/rnx"
