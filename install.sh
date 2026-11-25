#!/bin/sh
# Install a release binary, an explicitly supplied binary, or build this checkout.
set -eu

die() { printf 'bridgy installer: %s\n' "$*" >&2; exit 1; }
usage() {
    cat <<'HELP'
Usage: sh install.sh [--binary PATH | --from-source] [--bin-dir DIR]

Without --binary/--from-source, download a checksummed GitHub release.
Set BRIDGY_REPO=owner/repository and optionally BRIDGY_VERSION=v0.1.0.
Installs only the command. Run `bridgy install` separately to enable agent handoffs.
BRIDGY_HOME overrides HOME for the default bin directory (~/.local/bin).
No sudo, Python, Node, MCP server, or Rust toolchain is needed for release installs.
HELP
}

binary=
source_build=false
bridgy_home=${BRIDGY_HOME:-${HOME:-}}
[ -n "$bridgy_home" ] || die 'HOME is unset; set BRIDGY_HOME'
bin_dir=$bridgy_home/.local/bin
while [ "$#" -gt 0 ]; do
    case "$1" in
        --binary) [ "$#" -ge 2 ] || die '--binary requires a path'; binary=$2; shift 2 ;;
        --from-source) source_build=true; shift ;;
        --bin-dir) [ "$#" -ge 2 ] || die '--bin-dir requires a directory'; bin_dir=$2; shift 2 ;;
        --providers|--link-codex) die 'handoff options moved to `bridgy install`; install the command first' ;;
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
mkdir -p "$bin_dir"
[ -w "$bin_dir" ] || die "directory is not writable: $bin_dir"
[ ! -d "$bin_dir/bridgy" ] || die "destination is a directory: $bin_dir/bridgy"
# Stage beside the target so upgrades also work while an old executable is running.
staged=$(mktemp "$bin_dir/.bridgy-install.XXXXXXXX")
trap 'rm -f -- "$staged"; if [ -n "${temp_dir:-}" ]; then rm -rf -- "$temp_dir"; fi' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
cp "$binary" "$staged"
chmod 755 "$staged"
mv -f "$staged" "$bin_dir/bridgy"
printf '\nInstalled command: %s/bridgy\n' "$bin_dir"

# Quote paths as shell literals, including spaces, apostrophes, and dollar signs.
shell_quote() {
    printf "'"
    printf '%s' "$1" | sed "s/'/'\\\\''/g"
    printf "'"
}
case ":${PATH:-}:" in
    *":$bin_dir:"*) printf 'The installation directory is already on PATH.\n' ;;
    *)
        quoted_dir=$(shell_quote "$bin_dir")
        printf '\nThis directory is not on PATH. Run:\n'
        case "${SHELL:-sh}" in
            */fish)
                printf '  fish_add_path --universal %s\n' "$quoted_dir"
                ;;
            *)
                setup="export PATH=$quoted_dir:\"\$PATH\""
                printf '  %s\n' "$setup"
                printf 'For fresh terminals, also run:\n'
                case "${SHELL:-sh}" in
                    */zsh) rc=${ZDOTDIR:-${HOME:-$bridgy_home}}/.zshrc ;;
                    */bash)
                        rc=${HOME:-$bridgy_home}/.bashrc
                        login_rc=${HOME:-$bridgy_home}/.profile
                        for name in .bash_profile .bash_login; do
                            if [ -f "${HOME:-$bridgy_home}/$name" ]; then
                                login_rc=${HOME:-$bridgy_home}/$name
                                break
                            fi
                        done
                        printf "  printf '%%s\\\\n' %s >> %s\n" "$(shell_quote "$setup")" "$(shell_quote "$login_rc")"
                        ;;
                    *) rc=${HOME:-$bridgy_home}/.profile ;;
                esac
                printf "  printf '%%s\\\\n' %s >> %s\n" "$(shell_quote "$setup")" "$(shell_quote "$rc")"
                ;;
        esac
        ;;
esac
printf '\nOpen a fresh terminal, then run: bridgy --version\nLaunch the browser: bridgy\nOptional agent handoffs: bridgy install\n'
