#!/usr/bin/env bash
# Builds this checkout as a release binary, installs it over the direct Herdr
# install, and hands the running server off to it. Local use only: this
# replaces `herdr update` for a checkout that carries the tab swipe patch.
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
install_dir=${HERDR_INSTALL_DIR:-$HOME/.local/bin}
installed=$install_dir/herdr
record=$install_dir/herdr.local-build

# The vendored libghostty-vt needs the Zig version its build.zig.zon pins.
# PATH carries an older Zig on this machine, so default to the Homebrew keg.
if [ -z "${ZIG:-}" ] && [ -x /opt/homebrew/opt/zig@0.16/bin/zig ]; then
  export ZIG=/opt/homebrew/opt/zig@0.16/bin/zig
fi

cd "$repo"
cargo build --release --locked
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

cp "$built" "$installed.new"
mv -f "$installed.new" "$installed"
printf 'tag=%s\ncommit=%s\nversion=%s\nprotocol=%s\nbuilt=%s\n' \
  "$tag" "$commit" "$version" "$protocol" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" > "$record"
echo "installed herdr $version from $tag+$commit at $installed"

handoff="herdr server live-handoff --import-exe $installed --expected-protocol $protocol --expected-version $version"
if [ -n "${HERDR_ENV:-}" ]; then
  echo "inside Herdr: detach, then run this from a plain terminal:"
  echo "  $handoff"
  exit 0
fi
if herdr status server 2>/dev/null | grep -q '^status: running'; then
  $handoff
  herdr status server
  echo "detach and reattach so the client is the new binary too"
else
  echo "no server is running; the next herdr launch starts the new one"
fi
