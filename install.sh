#!/bin/sh
# Install an explicitly versioned native release. No Rust toolchain required.
set -eu

version=${1:-v0.2.0}
case "$version" in
  v[0-9]*.[0-9]*.[0-9]*) ;;
  *) echo 'Usage: sh install.sh [vVERSION] (for example v0.2.0)' >&2; exit 1 ;;
esac
case "$version" in *[!a-zA-Z0-9.-]*) echo 'Invalid version' >&2; exit 1 ;; esac
case "$(uname -s)/$(uname -m)" in
  Linux/x86_64) target=x86_64-unknown-linux-gnu ;;
  Linux/aarch64|Linux/arm64) target=aarch64-unknown-linux-gnu ;;
  Darwin/x86_64) target=x86_64-apple-darwin ;;
  Darwin/arm64) target=aarch64-apple-darwin ;;
  *) echo 'Supported platforms: Linux x86_64/arm64 and macOS Intel/Apple Silicon.' >&2; exit 1 ;;
esac
install_dir=${SYMSLICE_INSTALL_DIR:-"$HOME/.local/bin"}
case "$install_dir" in /*) ;; *) echo 'SYMSLICE_INSTALL_DIR must be absolute' >&2; exit 1 ;; esac
command -v curl >/dev/null || { echo 'curl is required' >&2; exit 1; }
command -v tar >/dev/null || { echo 'tar is required' >&2; exit 1; }
archive="symslice-mcp-$version-$target.tar.gz"
base="https://github.com/olegpustovit/symslice-mcp/releases/download/$version"
work_dir=$(mktemp -d)
staged=
cleanup() { rm -rf "$work_dir"; [ -z "$staged" ] || rm -f "$staged"; }
trap cleanup EXIT HUP INT TERM
curl --fail --location --silent --show-error --retry 3 "$base/$archive" -o "$work_dir/$archive"
curl --fail --location --silent --show-error --retry 3 "$base/$archive.sha256" -o "$work_dir/$archive.sha256"
(
  cd "$work_dir"
  if command -v sha256sum >/dev/null; then
    sha256sum -c "$archive.sha256"
  else
    shasum -a 256 -c "$archive.sha256"
  fi
)
tar -xzf "$work_dir/$archive" -C "$work_dir" symslice-mcp
chmod 755 "$work_dir/symslice-mcp"
# Validate platform compatibility before replacing an existing installation.
actual=$("$work_dir/symslice-mcp" --version)
[ "$actual" = "symslice-mcp ${version#v}" ] || { echo "Unexpected binary version: $actual" >&2; exit 1; }
mkdir -p "$install_dir"
staged=$(mktemp "$install_dir/.symslice-mcp.XXXXXX")
cp "$work_dir/symslice-mcp" "$staged"
chmod 755 "$staged"
mv -f "$staged" "$install_dir/symslice-mcp"
staged=
printf 'Installed %s at %s/symslice-mcp\n' "$version" "$install_dir"
printf 'Use this absolute executable path in your MCP client configuration.\n'
