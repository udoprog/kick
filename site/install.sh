#!/bin/sh
# Installs the latest prebuilt kick into ${KICK_INSTALL_DIR:-$HOME/.local/bin}.
#
#   curl -fsSL https://udoprog.github.io/kick/install.sh | sh
#
# KICK_VERSION pins a release (the dated tag, e.g. 2026-05-04) instead of the latest.
# Windows is not supported by this script; download the zip from the releases.

set -eu

repo="udoprog/kick"
api="https://api.github.com/repos/$repo/releases"

say() {
    printf '%s\n' "kick-install: $*"
}

die() {
    printf '%s\n' "kick-install: error: $*" >&2
    exit 1
}

# Prints the <arch>-<os> pair for this machine, or fails.
detect_platform() {
    os=$(uname -s)
    machine=$(uname -m)

    case "$os" in
    Linux) os=linux ;;
    Darwin) os=macos ;;
    MINGW* | MSYS* | CYGWIN*) die "this script does not support Windows; use the zip from https://github.com/$repo/releases" ;;
    *) die "no prebuilt kick for $machine-$os" ;;
    esac

    case "$machine" in
    x86_64 | amd64) arch=x86_64 ;;
    aarch64 | arm64) arch=aarch64 ;;
    *) die "no prebuilt kick for $machine-$os" ;;
    esac

    # The platforms releases are built for.
    case "$arch-$os" in
    x86_64-linux | aarch64-macos) ;;
    *) die "no prebuilt kick for $arch-$os" ;;
    esac

    printf '%s\n' "$arch-$os"
}

# Reads a URL to stdout, or to the file given as the second argument.
fetch() {
    if command -v curl >/dev/null 2>&1; then
        if [ $# -gt 1 ]; then
            curl -fsSL -o "$2" "$1"
        else
            curl -fsSL "$1"
        fi
    elif command -v wget >/dev/null 2>&1; then
        if [ $# -gt 1 ]; then
            wget -q -O "$2" "$1"
        else
            wget -q -O - "$1"
        fi
    else
        die "need curl or wget"
    fi
}

# Picks the download URL ending in -<platform>.tar.gz from release JSON on stdin.
pick_asset() {
    grep -o '"browser_download_url": *"[^"]*"' |
        sed 's/^[^:]*: *"//; s/"$//' |
        grep -e "-$1\\.tar\\.gz\$" |
        head -n 1
}

# main
command -v tar >/dev/null 2>&1 || die "need tar"

platform=$(detect_platform)
dir=${KICK_INSTALL_DIR:-$HOME/.local/bin}

if [ -n "${KICK_VERSION:-}" ]; then
    release_url="$api/tags/$KICK_VERSION"
else
    release_url="$api/latest"
fi

say "looking up release ${KICK_VERSION:-latest}"
release=$(fetch "$release_url") || die "could not read $release_url"
url=$(printf '%s\n' "$release" | pick_asset "$platform") || true
[ -n "$url" ] || die "no prebuilt kick for $platform in release ${KICK_VERSION:-latest}"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM

say "downloading $url"
fetch "$url" "$tmp/kick.tar.gz" || die "download failed"
tar -xzf "$tmp/kick.tar.gz" -C "$tmp" kick || die "archive has no kick binary"

mkdir -p "$dir"
mv "$tmp/kick" "$dir/kick"
chmod +x "$dir/kick"
say "installed $dir/kick"

case ":$PATH:" in
*":$dir:"*) ;;
*) say "warning: $dir is not on your PATH; add it to your shell profile" ;;
esac
