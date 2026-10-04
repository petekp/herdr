#!/usr/bin/env bash
# Builds this checkout of the tab-swipe branch as a release binary and
# installs it over a direct Herdr install. Replaces `herdr update`, which
# refuses to run on this build so it cannot put stock Herdr back by accident.
#
#   scripts/tab-swipe-install.sh            build, back up stock, install
#   scripts/tab-swipe-install.sh --handoff  also hand the running server off
#
# Run it from a plain terminal, not from a pane inside Herdr.
set -euo pipefail

handoff=false
for arg in "$@"; do
  case "$arg" in
    --handoff) handoff=true ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

repo=$(cd "$(dirname "$0")/.." && pwd)
install_dir=${HERDR_INSTALL_DIR:-$HOME/.local/bin}
installed=$install_dir/herdr
record=$install_dir/herdr.local-build

command -v cargo >/dev/null || { echo "cargo not found; install Rust from https://rustup.rs" >&2; exit 1; }

# The vendored libghostty-vt needs the Zig minor pinned in
# vendor/libghostty-vt/build.zig.zon. On macOS with Homebrew, prefer that keg
# so an older zig on PATH does not get used.
if [ -z "${ZIG:-}" ] && [ -x /opt/homebrew/opt/zig@0.16/bin/zig ]; then
  export ZIG=/opt/homebrew/opt/zig@0.16/bin/zig
fi

cd "$repo"
HERDR_BUILD_VARIANT=tab-swipe cargo build --release --locked
built=$repo/target/release/herdr

version=$("$built" --version | awk '{print $2}')
protocol=$(sed -n 's/^pub const PROTOCOL_VERSION: u32 = \([0-9]*\);/\1/p' src/protocol/wire.rs)
commit=$(git rev-parse --short=8 HEAD)$(git diff --quiet HEAD || echo -dirty)
tag=$(git describe --tags --abbrev=0 --match 'v*' 2>/dev/null || echo unknown)

# Keep one copy of each stock release. A binary this script installed is
# recorded, so it is only backed up when its version is not the recorded one.
if [ -x "$installed" ]; then
  installed_version=$("$installed" --version | awk '{print $2}')
  recorded_version=$(sed -n 's/^version=//p' "$record" 2>/dev/null || true)
  backup=$install_dir/herdr-$installed_version-stock
  if [ ! -e "$backup" ] && [ "$installed_version" != "$recorded_version" ]; then
    cp -p "$installed" "$backup"
    echo "saved stock $installed_version as $backup"
  fi
fi

mkdir -p "$install_dir"
cp "$built" "$installed.new"
mv -f "$installed.new" "$installed"
printf 'tag=%s\ncommit=%s\nversion=%s\nprotocol=%s\nbuilt=%s\n' \
  "$tag" "$commit" "$version" "$protocol" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" > "$record"
echo "installed $("$installed" --version) from $tag+$commit at $installed"

on_path=$(command -v herdr || true)
if [ -n "$on_path" ] && [ "$on_path" != "$installed" ]; then
  echo "warning: 'herdr' on PATH is $on_path, not $installed." >&2
  echo "         remove that install (for Homebrew: brew uninstall herdr) or put $install_dir first in PATH." >&2
fi

handoff_command="herdr server live-handoff --import-exe $installed --expected-protocol $protocol --expected-version $version"
if [ -n "${HERDR_ENV:-}" ]; then
  echo "running inside Herdr: detach, then from a plain terminal either"
  echo "  $handoff_command"
  echo "to keep pane programs running, or stop Herdr and start it again."
  exit 0
fi
if ! herdr status server 2>/dev/null | grep -q '^status: running'; then
  echo "no server is running; the next herdr launch uses the new build"
  exit 0
fi
if [ "$handoff" = true ]; then
  $handoff_command
  herdr status server
  echo "detach and reattach so the client is the new build too"
else
  echo "a server is still running the old build. Either"
  echo "  $handoff_command"
  echo "to keep pane programs running (Herdr marks live handoff experimental),"
  echo "or stop Herdr and start it again."
fi
