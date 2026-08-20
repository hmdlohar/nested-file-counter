#!/usr/bin/env sh
# hnfc installer — curl | sh friendly
#   curl -fsSL https://raw.githubusercontent.com/<OWNER>/<REPO>/main/install.sh | sh
#   curl -fsSL .../install.sh | sh -s -- --uninstall
#   HNFC_VERSION=v0.1.0 sh install.sh            # pin version
#   HNFC_INSTALL_DIR=/usr/local/bin sh install.sh # custom dir
#   GH_REPO=owner/repo sh install.sh             # override repo
set -eu

REPO="${GH_REPO:-${HNFC_REPO:-}}" # e.g. "myorg/hnfc"
# Default repo — CHANGE THIS to your GitHub repo before publishing:
if [ -z "$REPO" ]; then
  REPO="YOUR_GH_USER/nested-file-counter"
  echo "warn: using placeholder repo $REPO — set GH_REPO=owner/repo or edit install.sh" >&2
fi

VERSION="${HNFC_VERSION:-${1:-}}"
INSTALL_DIR="${HNFC_INSTALL_DIR:-${INSTALL_DIR:-}}"
# handle --uninstall passed as first arg when piped via `sh -s -- --uninstall`
for arg in "$@"; do
  case "$arg" in
    --uninstall) VERSION="__uninstall__" ;;
    v*|latest) VERSION="$arg" ;;
  esac
done
if [ "${1:-}" = "--uninstall" ]; then VERSION="__uninstall__"; fi

# --- uninstall ---
do_uninstall() {
  BIN=""
  if [ -n "$INSTALL_DIR" ] && [ -f "$INSTALL_DIR/hnfc" ]; then BIN="$INSTALL_DIR/hnfc"; fi
  if [ -z "$BIN" ] && [ -f "$HOME/.local/bin/hnfc" ]; then BIN="$HOME/.local/bin/hnfc"; fi
  if [ -z "$BIN" ] && command -v hnfc >/dev/null 2>&1; then BIN="$(command -v hnfc)"; fi
  # also try hnfc --uninstall if binary supports it
  if command -v hnfc >/dev/null 2>&1; then
    if hnfc --uninstall 2>/dev/null; then exit 0; fi
  fi
  if [ -n "$BIN" ] && [ -f "$BIN" ]; then
    echo "Removing $BIN"
    rm -f "$BIN"
    # Windows exe
    rm -f "${BIN}.exe" 2>/dev/null || true
    echo "hnfc uninstalled."
  else
    echo "hnfc not found. Tried: \$HNFC_INSTALL_DIR/hnfc, ~/.local/bin/hnfc, \$(which hnfc)" >&2
    exit 1
  fi
  exit 0
}

if [ "$VERSION" = "__uninstall__" ]; then
  do_uninstall
fi

# allow `hnfc --uninstall` delegation if user runs install.sh after install
if [ "${HNFC_UNINSTALL:-}" = "1" ]; then do_uninstall; fi

# --- detect OS/arch ---
OS="$(uname -s 2>/dev/null || echo unknown)"
ARCH="$(uname -m 2>/dev/null || echo unknown)"

case "$OS" in
  Linux) OS="linux" ;;
  Darwin) OS="darwin" ;;
  MINGW*|MSYS*|CYGWIN*|Windows_NT) OS="windows" ;;
  *) echo "Unsupported OS: $OS" >&2; exit 1 ;;
esac

case "$ARCH" in
  x86_64|amd64) ARCH="amd64" ;;
  aarch64|arm64) ARCH="arm64" ;;
  *) echo "Unsupported arch: $ARCH (only amd64/arm64)" >&2; exit 1 ;;
esac

# asset naming must match release workflow below
# linux:  hnfc-linux-amd64, hnfc-linux-arm64
# darwin: hnfc-darwin-amd64, hnfc-darwin-arm64
# windows: hnfc-windows-amd64.exe  (or .zip)
if [ "$OS" = "windows" ]; then
  if [ "$ARCH" != "amd64" ]; then echo "Windows arm64 not yet published" >&2; exit 1; fi
  ASSET="hnfc-windows-amd64.zip"
  BIN_NAME="hnfc.exe"
else
  ASSET="hnfc-${OS}-${ARCH}.tar.gz"
  BIN_NAME="hnfc"
fi

# resolve version
if [ -z "$VERSION" ] || [ "$VERSION" = "latest" ]; then
  if command -v curl >/dev/null 2>&1; then
    VERSION="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep -o '"tag_name": *"[^"]*"' | head -1 | cut -d'"' -f4)"
  elif command -v wget >/dev/null 2>&1; then
    VERSION="$(wget -qO- "https://api.github.com/repos/${REPO}/releases/latest" | grep -o '"tag_name": *"[^"]*"' | head -1 | cut -d'"' -f4)"
  fi
  if [ -z "${VERSION:-}" ]; then
    echo "Could not resolve latest release. Set HNFC_VERSION=v0.1.0" >&2
    exit 1
  fi
fi

case "$VERSION" in v*) ;; *) VERSION="v$VERSION" ;; esac

URL="https://github.com/${REPO}/releases/download/${VERSION}/${ASSET}"

# choose install dir
if [ -z "$INSTALL_DIR" ]; then
  if [ -w /usr/local/bin ] 2>/dev/null && [ "$OS" != "darwin" ]; then
    # prefer ~/.local/bin for curl|sh (no sudo); use /usr/local/bin only if explicitly set
    INSTALL_DIR="$HOME/.local/bin"
  else
    INSTALL_DIR="$HOME/.local/bin"
  fi
fi

# allow HNFC_INSTALL_DIR override to be a file path
case "$INSTALL_DIR" in
  */hnfc|*/hnfc.exe) INSTALL_DIR="$(dirname "$INSTALL_DIR")" ;;
esac

echo "Installing hnfc $VERSION ($OS/$ARCH) from $URL"
echo "  -> $INSTALL_DIR/$BIN_NAME"

TMP="$(mktemp -d 2>/dev/null || mktemp -d -t hnfc)"
trap 'rm -rf "$TMP"' EXIT INT TERM

if command -v curl >/dev/null 2>&1; then
  curl -fsSL "$URL" -o "$TMP/$ASSET"
elif command -v wget >/dev/null 2>&1; then
  wget -qO "$TMP/$ASSET" "$URL"
else
  echo "Need curl or wget" >&2; exit 1
fi

mkdir -p "$INSTALL_DIR"

case "$ASSET" in
  *.tar.gz) tar -xzf "$TMP/$ASSET" -C "$TMP";;
  *.zip)
    if command -v unzip >/dev/null 2>&1; then unzip -q "$TMP/$ASSET" -d "$TMP"
    else echo "Need unzip for Windows asset" >&2; exit 1; fi
    ;;
esac

# asset contains either `hnfc` or `hnfc.exe` at top level
BIN_SRC="$(find "$TMP" -maxdepth 3 -type f -name "hnfc*" | head -1)"
if [ -z "$BIN_SRC" ]; then echo "Archive did not contain hnfc binary" >&2; ls -R "$TMP" >&2; exit 1; fi

install -m 755 "$BIN_SRC" "$INSTALL_DIR/$BIN_NAME" 2>/dev/null || cp -f "$BIN_SRC" "$INSTALL_DIR/$BIN_NAME" && chmod +x "$INSTALL_DIR/$BIN_NAME"

echo "Installed to $INSTALL_DIR/$BIN_NAME"
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) echo "NOTE: $INSTALL_DIR is not on PATH. Add to your shell rc:"; echo "  export PATH=\"\$HOME/.local/bin:\$PATH\"";;
esac

# verify
if command -v hnfc >/dev/null 2>&1; then hnfc --help | head -5; else "$INSTALL_DIR/$BIN_NAME" --help | head -5; fi

echo "To uninstall: hnfc --uninstall  or  curl -fsSL https://raw.githubusercontent.com/${REPO}/main/install.sh | sh -s -- --uninstall"
