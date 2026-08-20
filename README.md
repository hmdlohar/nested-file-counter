# hnfc — ncdu for file counts

`hnfc` recursively counts files per directory and shows an **ncdu-like TUI** (bars, drill-down, live progress) or a sorted one-shot listing. Also shows **size** (`KB/MB/GB`) + counts with commas and `k/M/B` shorthands. Pure Rust, single static binary.

> Default: **hidden files included**, **stays on one filesystem** (like `ncdu -x`) — mounted drives are skipped unless you ask.

## Install

### curl | sh (recommended)

```sh
curl -fsSL https://raw.githubusercontent.com/hmdlohar/nested-file-counter/main/install.sh | sh
# custom version / repo / dir
HNFC_VERSION=v0.1.0 sh install.sh
GH_REPO=OWNER/REPO sh install.sh
HNFC_INSTALL_DIR=/usr/local/bin sh install.sh
```

Installs to `~/.local/bin/hnfc` (add it to `PATH` if needed):

```sh
export PATH="$HOME/.local/bin:$PATH"
```

### From a GitHub Release tarball/zip

```sh
# Linux amd64 example
curl -fsSLO https://github.com/hmdlohar/nested-file-counter/releases/download/v0.1.0/hnfc-linux-amd64.tar.gz
tar -xzf hnfc-linux-amd64.tar.gz
install -m 755 hnfc ~/.local/bin/hnfc
hnfc --help
```

### From source (Docker — no host Rust needed)

```sh
make build-release   # or: make build  (dev)
./target/release/hnfc --help
```

## Usage

```sh
hnfc                  # TUI of current dir
hnfc /var/log         # TUI of path
hnfc --sort size      # sort by disk size instead of count
hnfc --no-tui --top 30
hnfc --no-hidden --exclude '*.log' --exclude 'node_modules'
hnfc --cross-filesystem /   # include mounted/other filesystems
hnfc --depth 2 .            # limit recursion depth
```

### Flags

| Flag | Default | Notes |
|------|---------|-------|
| `--hidden` / `--no-hidden` | **on** | Dotfiles |
| `-x, --one-filesystem` | **on** | Stay on one filesystem |
| `-X, --cross-filesystem` | off | Cross mount points |
| `--sort count\|size` | `count` | In TUI: `s` toggles |
| `-e, --exclude GLOB` | — | Repeatable, matches name & path |
| `--depth N` | — | 0 = root only |
| `--no-tui --top N` | 20 | Print listing and exit |
| `--uninstall` | — | Remove installed binary |

### TUI keys

`↑/↓` or `j/k` navigate · `→`/`Enter`/`l` enter · `←`/`Backspace`/`Esc` up · `s` sort (count/size) · `c`/`y` copy path (OSC52 + `wl-copy`/`xclip`/`xsel`/`pbcopy`) · `D` delete (confirmation modal, recalculates counts) · `r` rescan · `q` quit (only `q` quits — `Esc` goes up).

## Uninstall

```sh
hnfc --uninstall
# or
curl -fsSL https://raw.githubusercontent.com/hmdlohar/nested-file-counter/main/install.sh | sh -s -- --uninstall
# or
rm ~/.local/bin/hnfc
```

## Releasing

Use the helper script — it bumps `Cargo.toml`/`Cargo.lock`, commits, tags, and pushes (which triggers the release workflow):

```sh
./scripts/release.sh 0.2.0          # 0.2.0 or v0.2.0
./scripts/release.sh 0.2.0 --dry-run
./scripts/release.sh 0.2.0 --no-push  # commit+tag locally, push later
```

Or manually:

```sh
git tag v0.1.0 && git push origin v0.1.0
```

The workflow builds **linux (amd64/arm64 musl), macOS (amd64/arm64), Windows (amd64)** and attaches `hnfc-*` tarballs/zips to the GitHub Release.

Assets use these names (consumed by `install.sh`):

- `hnfc-linux-amd64.tar.gz`, `hnfc-linux-arm64.tar.gz`
- `hnfc-darwin-amd64.tar.gz`, `hnfc-darwin-arm64.tar.gz`
- `hnfc-windows-amd64.zip`

## License

MIT
