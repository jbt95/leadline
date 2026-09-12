#!/bin/sh
# leadline installer.
#
#   curl -fsSL https://raw.githubusercontent.com/jbt95/leadline/main/install.sh | sh
#
# Downloads the release archive for this machine, verifies it against the
# release SHA256SUMS, and installs the binary. No sudo, no config edits.
#
# Environment:
#   LEADLINE_VERSION      Release tag to install (default: latest)
#   LEADLINE_INSTALL_DIR  Destination directory (default: ~/.local/bin)
#   LEADLINE_BASE_URL     Download origin (default: https://github.com/jbt95/leadline)
#   LEADLINE_SKIP_CHECKSUM=1  Skip SHA256 verification (not recommended)
#
# Windows users: download leadline-x86_64-pc-windows-msvc.zip from
# https://github.com/jbt95/leadline/releases instead.
set -eu

usage() {
    cat <<'EOF'
leadline installer.

  curl -fsSL https://raw.githubusercontent.com/jbt95/leadline/main/install.sh | sh

Downloads the release archive for this machine, verifies it against the
release SHA256SUMS, and installs the binary. No sudo, no config edits.

Environment:
  LEADLINE_VERSION          Release tag to install (default: latest)
  LEADLINE_INSTALL_DIR      Destination directory (default: ~/.local/bin)
  LEADLINE_BASE_URL         Download origin (default: https://github.com/jbt95/leadline)
  LEADLINE_SKIP_CHECKSUM=1  Skip SHA256 verification (not recommended)

Windows users: download leadline-x86_64-pc-windows-msvc.zip from
https://github.com/jbt95/leadline/releases instead.
EOF
}

case "${1:-}" in
    --help | -h)
        usage
        exit 0
        ;;
esac

BASE_URL="${LEADLINE_BASE_URL:-https://github.com/jbt95/leadline}"
INSTALL_DIR="${LEADLINE_INSTALL_DIR:-${HOME:?HOME is not set}/.local/bin}"

os=$(uname -s)
arch=$(uname -m)
case "$os" in
    Darwin)
        case "$arch" in
            arm64 | aarch64) target="aarch64-apple-darwin" ;;
            x86_64 | amd64) target="x86_64-apple-darwin" ;;
            *)
                echo "error: unsupported macOS architecture '$arch'" >&2
                exit 1
                ;;
        esac
        ;;
    Linux)
        case "$arch" in
            aarch64 | arm64) target="aarch64-unknown-linux-gnu" ;;
            x86_64 | amd64) target="x86_64-unknown-linux-gnu" ;;
            *)
                echo "error: unsupported Linux architecture '$arch'" >&2
                exit 1
                ;;
        esac
        ;;
    *)
        echo "error: unsupported operating system '$os'." >&2
        echo "Download a release archive from $BASE_URL/releases" >&2
        exit 1
        ;;
esac

archive="leadline-$target.tar.gz"
if [ -n "${LEADLINE_VERSION:-}" ]; then
    release_url="$BASE_URL/releases/download/$LEADLINE_VERSION"
else
    release_url="$BASE_URL/releases/latest/download"
fi

if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -q "$1" -O "$2"; }
else
    echo "error: curl or wget is required" >&2
    exit 1
fi

tmp=$(mktemp -d "${TMPDIR:-/tmp}/leadline.XXXXXX")
trap 'rm -rf "$tmp"' EXIT INT TERM

echo "Downloading $archive..."
if ! fetch "$release_url/$archive" "$tmp/$archive"; then
    echo "error: cannot download $release_url/$archive" >&2
    echo "Check the tag (LEADLINE_VERSION) and that a release exists." >&2
    exit 1
fi

if [ "${LEADLINE_SKIP_CHECKSUM:-}" = "1" ]; then
    echo "warning: skipping SHA256 verification" >&2
else
    if ! fetch "$release_url/SHA256SUMS" "$tmp/SHA256SUMS"; then
        echo "error: cannot download $release_url/SHA256SUMS" >&2
        echo "Set LEADLINE_SKIP_CHECKSUM=1 to install without verification." >&2
        exit 1
    fi
    # SHA256SUMS lines are "<hash>  <path>"; tolerate binary-mode '*' markers
    # and directory prefixes (older releases wrote "dist/<name>").
    expected=$(awk -v name="$archive" '{
        file = $2
        sub(/^\*/, "", file)
        sub(/^.*\//, "", file)
        if (file == name) { print $1; exit }
    }' "$tmp/SHA256SUMS")
    if [ -z "$expected" ]; then
        echo "error: $archive is not listed in SHA256SUMS" >&2
        exit 1
    fi
    if command -v sha256sum >/dev/null 2>&1; then
        actual=$(sha256sum "$tmp/$archive" | awk '{ print $1 }')
    elif command -v shasum >/dev/null 2>&1; then
        actual=$(shasum -a 256 "$tmp/$archive" | awk '{ print $1 }')
    else
        echo "error: sha256sum or shasum is required to verify the download" >&2
        echo "Set LEADLINE_SKIP_CHECKSUM=1 to install without verification." >&2
        exit 1
    fi
    if [ "$expected" != "$actual" ]; then
        echo "error: checksum mismatch for $archive" >&2
        echo "expected $expected" >&2
        echo "actual   $actual" >&2
        exit 1
    fi
fi

tar -xzf "$tmp/$archive" -C "$tmp"
if [ ! -f "$tmp/leadline" ]; then
    echo "error: archive does not contain the leadline binary" >&2
    exit 1
fi

mkdir -p "$INSTALL_DIR"
cp "$tmp/leadline" "$INSTALL_DIR/leadline"
chmod 755 "$INSTALL_DIR/leadline"

"$INSTALL_DIR/leadline" --version
echo "Installed to $INSTALL_DIR/leadline"
case ":${PATH:-}:" in
    *":$INSTALL_DIR:"*) ;;
    *)
        echo "Add it to PATH: export PATH=\"$INSTALL_DIR:\$PATH\""
        ;;
esac
