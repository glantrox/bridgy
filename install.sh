#!/bin/sh
# Install a release binary, an explicitly supplied binary, or build this checkout.
set -eu

die() { printf 'bridgy installer: %s\n' "$*" >&2; exit 1; }
usage() {
    cat <<'HELP'
Usage: sh install.sh [--binary PATH | --from-source] [--providers claude,codex] [--bin-dir DIR] [--link-codex]

Without --binary/--from-source, download a checksummed GitHub release.
Set BRIDGY_REPO=owner/repository and optionally BRIDGY_VERSION=v0.1.0.
BRIDGY_HOME overrides HOME for both provider configs and the default bin directory.
No sudo, Python, Node, MCP server, or Rust toolchain is needed for release installs.
HELP
}

binary=
source_build=false
providers=
link_codex=false
bridgy_home=${BRIDGY_HOME:-${HOME:-}}
[ -n "$bridgy_home" ] || die 'HOME is unset; set BRIDGY_HOME'
bin_dir=$bridgy_home/.local/bin
while [ "$#" -gt 0 ]; do
    case "$1" in
        --binary) [ "$#" -ge 2 ] || die '--binary requires a path'; binary=$2; shift 2 ;;
        --from-source) source_build=true; shift ;;
        --providers) [ "$#" -ge 2 ] || die '--providers requires a list'; providers=$2; shift 2 ;;
        --bin-dir) [ "$#" -ge 2 ] || die '--bin-dir requires a directory'; bin_dir=$2; shift 2 ;;
        --link-codex) link_codex=true; shift ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown option: $1" ;;
    esac
done
[ "$source_build" = false ] || [ -z "$binary" ] || die '--binary and --from-source conflict'
case "$bridgy_home" in /*) ;; *) die 'HOME/BRIDGY_HOME must be absolute' ;; esac
case "$bin_dir" in /*) ;; *) die '--bin-dir must be absolute' ;; esac

if [ "$source_build" = true ]; then
    command -v cargo >/dev/null 2>&1 || die 'building from source requires Cargo; use a release binary instead'
    script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
    cargo build --manifest-path "$script_dir/Cargo.toml" --locked --release --target-dir "$script_dir/target"
    binary=$script_dir/target/release/bridgy
fi

if [ -z "$binary" ]; then
    repo=${BRIDGY_REPO:-}
    [ -n "$repo" ] || die 'set BRIDGY_REPO=owner/repository, or use --binary PATH / --from-source'
    case "$repo" in *[!a-zA-Z0-9_./-]*|/*|*..*) die 'invalid BRIDGY_REPO' ;; esac
    case "$repo" in */*) ;; *) die 'BRIDGY_REPO must be owner/repository' ;; esac
    case "$(uname -s)/$(uname -m)" in
        Darwin/arm64|Darwin/aarch64) target=aarch64-apple-darwin ;;
        Darwin/x86_64) target=x86_64-apple-darwin ;;
        Linux/x86_64) target=x86_64-unknown-linux-musl ;;
        Linux/aarch64|Linux/arm64) target=aarch64-unknown-linux-musl ;;
        *) die 'supported systems: macOS/Linux, x86_64/arm64' ;;
    esac
    command -v curl >/dev/null 2>&1 || die 'curl is required to download a release'
    if command -v sha256sum >/dev/null 2>&1; then checksum=sha256sum
    elif command -v shasum >/dev/null 2>&1; then checksum=shasum
    else die 'sha256sum or shasum is required to verify the download'; fi
    temp_dir=$(mktemp -d "${TMPDIR:-/tmp}/bridgy-install.XXXXXXXX")
    trap 'rm -rf -- "$temp_dir"' EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM
    version=${BRIDGY_VERSION:-latest}
    case "$version" in *[!a-zA-Z0-9._-]*) die 'invalid BRIDGY_VERSION' ;; esac
    if [ "$version" = latest ]; then base=https://github.com/$repo/releases/latest/download
    else base=https://github.com/$repo/releases/download/$version; fi
    asset=bridgy-$target
    curl --fail --show-error --silent --location --proto '=https' --proto-redir '=https' "$base/$asset" -o "$temp_dir/bridgy"
    curl --fail --show-error --silent --location --proto '=https' --proto-redir '=https' "$base/$asset.sha256" -o "$temp_dir/checksum"
    read -r expected ignored < "$temp_dir/checksum"
    [ "${#expected}" -eq 64 ] || die 'invalid SHA-256 checksum'
    case "$expected" in *[!a-fA-F0-9]*) die 'invalid SHA-256 checksum' ;; esac
    if [ "$checksum" = sha256sum ]; then actual=$(sha256sum "$temp_dir/bridgy")
    else actual=$(shasum -a 256 "$temp_dir/bridgy"); fi
    actual=${actual%% *}
    [ "$actual" = "$expected" ] || die 'checksum mismatch; refusing to execute download'
    chmod 755 "$temp_dir/bridgy"
    binary=$temp_dir/bridgy
fi

[ -f "$binary" ] && [ -x "$binary" ] || die "binary is missing or not executable: $binary"
set -- --home "$bridgy_home" install --bin-dir "$bin_dir"
if [ -n "$providers" ]; then set -- "$@" --providers "$providers"; fi
if [ "$link_codex" = true ]; then set -- "$@" --link-codex; fi
"$binary" "$@"
printf '\nInstalled Bridgy. Add this directory to PATH: %s\n' "$bin_dir"
