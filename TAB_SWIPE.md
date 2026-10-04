# Tab swipe for Herdr

This branch is Herdr as released, plus one feature: switching tabs with a
horizontal trackpad swipe or a horizontal mouse wheel. The focused tab's
highlight slides toward the neighbor as you swipe, and the pane content slides
with it. Past either end, the strip wraps around.

These instructions are written for a coding agent to follow. Paste the link to
this file into your agent and ask it to install the tab swipe build.

## Input devices

The swipe reads horizontal wheel events from your terminal, so it works with
whatever your terminal turns into those.

- Trackpads: a two-finger horizontal swipe. Built and tuned on a MacBook
  trackpad in Ghostty, including the momentum after you lift your fingers. A
  Magic Trackpad sends the same input.
- Magic Mouse: a sideways swipe on its surface arrives like a trackpad swipe.
  Not measured.
- Mice with a tilt wheel or a thumb wheel: the second tilt or notch switches a
  tab. A free-spinning thumb wheel sends a dense stream instead, which counts
  like a trackpad: about 48 events per switch.
- Any mouse over the tab row: a vertical wheel there swipes too, up for the
  previous tab and down for the next. Over a pane, a vertical wheel scrolls
  the pane as before.

Your terminal must report horizontal wheel events in SGR mouse mode. Ghostty
does. Without that, you keep the vertical wheel over the tab row. Herdr's
mouse capture, `ui.mouse_capture`, must be on, which is the default. The
timing is tuned to macOS trackpad event rates; Linux trackpads report at
other rates and have not been tried.

## What you need

- A direct Herdr install, the kind `curl -fsSL https://herdr.dev/install.sh | sh`
  makes, at `~/.local/bin/herdr`. If Herdr came from Homebrew, mise, or Nix,
  remove that install first, or whichever `herdr` comes first on `PATH` is the
  one that runs: `brew uninstall herdr`, `mise uninstall herdr`, or the Nix
  equivalent.
- Rust, through rustup. `rust-toolchain.toml` in this repository picks the
  toolchain version.
- Zig, at the minor version pinned by `minimum_zig_version` in
  `vendor/libghostty-vt/build.zig.zon` (0.16 as of October 2026). Herdr's
  terminal engine is built with it. On macOS: `brew install zig@0.16` and
  export `ZIG=/opt/homebrew/opt/zig@0.16/bin/zig`. Elsewhere, download that
  version from https://ziglang.org/download/ and point `ZIG` at the binary.
  The build fails with a clear message when the version is wrong.
- macOS or Linux. The code has no platform-specific parts, but nobody has
  built this branch for Windows yet.

## Install

```bash
git clone --branch tab-swipe https://github.com/petekp/herdr.git
cd herdr
scripts/tab-swipe-install.sh
```

The script builds the release binary, saves your stock Herdr as
`~/.local/bin/herdr-<version>-stock`, installs the new binary over
`~/.local/bin/herdr`, and writes what it built to
`~/.local/bin/herdr.local-build`. Run it from a plain terminal, not from a
pane inside Herdr.

Then restart Herdr so both the server and the client run the new build. The
script prints two ways: stop Herdr and start it again, or hand the running
server off with the `herdr server live-handoff` command it prints, which keeps
the programs in your panes running. Herdr marks live handoff experimental.

Check: `herdr --version` prints `herdr 0.9.3 (tab-swipe)` or later. Open a
workspace with two tabs and swipe sideways over the tab row.

## Settings

Both keys live under `[ui]` in `~/.config/herdr/config.toml` and default to
on. Reload with `herdr server reload-config`.

- `tab_swipe = false` turns the gesture off.
- `tab_swipe_over_panes = false` keeps the gesture to the tab row, so a
  program that scrolls sideways itself, such as an editor with wrapping off,
  gets the horizontal wheel again.

## Update

When Herdr shows an update, do not run `herdr update`. On this build it
refuses and points here, because it would otherwise install stock Herdr over
the tab swipe build. Instead:

```bash
cd herdr          # the clone from the install step
git pull
scripts/tab-swipe-install.sh
```

Then restart Herdr as above.

The branch follows Herdr's stable releases. `git describe --tags --abbrev=0 --match 'v*'`
in the clone shows which release it is on. Compare it with the latest tag at
https://github.com/herdrdev/herdr/releases. When the branch is behind, either
wait for the port or do it yourself, below.

## Port the branch to a newer Herdr release yourself

The feature is a single squashed commit on top of a release tag, so a port is
one three-way merge.

```bash
git remote add upstream https://github.com/herdrdev/herdr.git
git fetch upstream --tags
git switch -C tab-swipe-local vX.Y.Z      # the release you want
git merge --squash tab-swipe
```

Resolve conflicts, keeping both sides' intent: the swipe adds code to the
client shell's mouse handling, tab bar rendering, pane composition, and one
server method, `client_shell.surface.read`. Then run the checks Herdr's
`justfile` runs, with the `ZIG` variable set as above:

```bash
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo nextest run --locked
```

Commit, and run `scripts/tab-swipe-install.sh`.

## Go back to stock Herdr

```bash
curl -fsSL https://herdr.dev/install.sh | sh
```

That installs the current stock release over `~/.local/bin/herdr`. Restart
Herdr. The backup `~/.local/bin/herdr-<version>-stock` is also still there.

## Branch layout

- `tab-swipe`: the current port plus fixes, one squashed feature commit per
  release and ordinary commits on top. Pull this.
- `feat/tab-swipe`: the original development history. Not maintained.

Herdr is Apache-2.0 licensed; this branch keeps its license and notices.
