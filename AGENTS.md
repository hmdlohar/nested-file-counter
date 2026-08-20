# AGENTS.md — Guide for AI / Human Contributors

> Read this before touching code. It captures *how* hnfc is built so the next agent doesn't have to reverse-engineer history from git log.

## 1. What is hnfc?

**hnfc** (nested-file-counter) is an `ncdu`-like TUI that counts **files** per directory (not disk blocks) and also shows on-disk size. Single Rust binary (~1.6 MB stripped), cross-platform (Linux/musl, macOS, Windows).

- Default: **hidden files included**, **stays on one filesystem** (`-x` semantics). Mounted/virtual FSs are skipped unless `--cross-filesystem` / `-X`.
- Live progress with spinner + `current: <path>` during scan; one-shot mode via `--no-tui`.

Repo: `hmdlohar/nested-file-counter` · Binary name: `hnfc` · Edition 2021 · License MIT.

## 2. How Development Works (Pure Docker)

The user explicitly chose **Pure Docker** — no host `rustup`/`cargo`.

| Command | What it does |
|---------|--------------|
| `make image` | `docker build -t hnfc-dev .` (`rust:1-slim` + `pkg-config`) |
| `make build` / `make build-release` | `cargo build` / `cargo build --release` inside `hnfc-dev` with `-v $PWD:/work -v hnfc-cargo-registry:/usr/local/cargo/registry` |
| `make check` | `cargo check` |
| `make test` | `cargo test` (7 tests) |
| `make run ARGS="--help"` | `cargo run -- <args>` **with** `-v /:/host:ro` so scanning `/` works inside container |
| `make scan ARGS="/tmp"` | one-shot: `cargo run -- --no-tui --top 20 ...` |
| `make dist` | cross musl via `ghcr.io/rust-cross/cargo-zigbuild` |
| `docker run --rm -w /work -v "$PWD":/work -v hnfc-cargo-registry:/usr/local/cargo/registry --entrypoint bash hnfc-dev -lc 'export PATH=/usr/local/cargo/bin:$PATH; cargo check --target x86_64-pc-windows-msvc'` | verify Windows cross-compile without CI |

**Never** run bare `cargo` on host (it doesn't exist). Always wrap via `docker run ... hnfc-dev cargo ...` with `PATH=/usr/local/cargo/bin:$PATH` if you use `bash -lc`. The `Makefile:9` `DOCKER_RUN` variable is the canonical wrapper.

**Release flow** is `scripts/release.sh` (see §7). It is the *only* supported way to bump version — it edits `Cargo.toml` `[package] version` correctly (awk `-F'"'`) and `Cargo.lock`.

## 3. Architecture

### 3.1 Module map

```
src/
  main.rs   CLI wiring, --uninstall, atty detection, delegates to scan/tui/output
  cli.rs    clap Parser + ValueEnum (SortArg). Defaults enforced here & in scan.rs::Default
  scan.rs   CORE: Node tree, ScanOptions, ScanProgress, sort helpers, iterative walker
  fmt.rs    Pure formatting: comma, compact, human_bytes, truncate_left — all with tests
  output.rs One-shot sorted listing (collect_dirs + sort_by SortBy)
  tui.rs    Ratatui+ crossterm TUI: App state, scan_with_live_progress, draw_*, copy/delete
```

### 3.2 Data flow

```
Cli::parse()
  -> SortBy + include_hidden/one_filesystem resolved (main.rs:30-42)
  -> ScanOptions { root (canonicalized), include_hidden, exclude_patterns, max_depth, one_filesystem, sort }
  -> if --no-tui or !is_terminal:  scan(&opts) or scan_with_progress(|p| write!(stderr,...)) -> output::print_one_shot
  -> else:                         tui::run_tui(opts)
            -> scan_with_live_progress (spawns scan_with_progress in thread, polls mpsc + draws spinner/progress)
            -> App::new(root, opts) -> run_loop (terminal.draw + event::poll)
```

### 3.3 Scanner (`scan.rs`) — the most load-bearing file

- **Iterative stack, not recursion** (`Frame` + `stack: Vec<Frame>`). Avoids stack overflow on deep trees.
- Each `Frame` holds `direct`/`direct_bytes` (files directly in that dir) + `children: Vec<Node>` + `entries: Vec<DirEntry>` + `idx`.
- **Phase 1** for each dir: `read_entries(path)` → `Vec<DirEntry>` + `Option<String>` error.
- **Phase 2**: walk `entries[idx++]` one by one:
  - Skip if `is_hidden(name)` and `!include_hidden`.
  - Skip if `should_exclude(path,name,globset)` (globset built from `GlobBuilder::literal_separator(true)`).
  - `file_type().ok()` == None → count as file (fallback).
  - `is_symlink()` → count as file, don't follow (prevents double-count / loops).
  - `is_dir()`:
    - Check `max_depth` (`depth+1 > max` → leaf with "max depth reached").
    - Check pseudo-FS (`is_pseudo_fs` → `/proc /sys /dev /run /snap` plus `/host/*` Docker translation) *only when not `one_filesystem`* — these are always virtual noise.
    - **Filesystem boundary** (`cfg(unix)` only): capture `root_dev = fs::metadata(root).dev()` if `one_filesystem`. For each subdir `fs::metadata(path).dev() != root_dev` → leaf with "different filesystem". On `not(unix)` this whole block is compiled out; `let _ = &root_dev` silences unused warning. **This caused Windows CI failure before `#[cfg(unix)]` gating** — never use `std::os::unix` unconditionally.
    - Cycle detection: `visited: HashSet<PathBuf>` of `canonicalize()` results; `!visited.insert(canon)` → "cycle detected".
    - Otherwise push new `Frame` and continue (emits progress).
  - else (regular file): `fs::metadata(path).len()` → `direct_bytes`, `progress.bytes`.
- **Phase 3**: when `idx >= entries.len()`, pop `Frame`, compute `total = direct + sum(child.total)` and `total_bytes` similarly, `sort_children(&mut children, opts.sort)`, create `Node`, push into `parent.children`.
- **Progress** (`ScanProgress { dirs, files, bytes, current }`): `on_progress` called at most every 60 ms (`emit_every`), plus immediately at start. `dirs` bumped on push; `current` set to dir path on push. Frontend draws `dirs/files/bytes + current`.
- **Sorting**: `SortBy::Count` vs `Size`, `sort_children` / `sort_tree` helpers. The tree is sorted at construction (pop phase) and re-sorted after delete or `s` toggle.

`Node` fields: `name, path, is_dir, direct, total, direct_bytes, total_bytes, children, error`.

**Tests** (in `scan.rs:419`): `counts_nested`, `hidden_included_by_default`, `exclude_glob`, `max_depth` — assert totals/direct and hidden default. Must stay passing on Linux and Windows.

### 3.4 Formatter (`fmt.rs`)

Pure functions — no I/O:
- `comma(u64)` → `"1,234,567"`
- `compact(u64)` → `"1.2M"`, `"12k"`, `"1.0B"`
- `human_bytes(u64)` → `"1.5 KB"`, `"2.0 MB"` (1024-based)
- `truncate_left(&str, max_chars)` → `"…tail"` keeps tail visible for long paths

### 3.5 TUI (`tui.rs`)

- `App { root, path: Vec<usize> (breadcrumb indices), selected, opts, sort, status, pending_delete }`
- `current_node()` walks `path`; `selected_node()` is `current_node().children[selected]`.
- **Startup**: `run_tui(opts)` → `enable_raw_mode + EnterAlternateScreen` → `scan_with_live_progress` (thread + two `mpsc` channels) → `App::new` → `run_loop` → cleanup `disable_raw_mode + LeaveAlternateScreen`.
- `scan_with_live_progress`: draws `draw_scanning` (spinner `SPINNER[]`, stats `dirs/files/bytes`, left-aligned `current: shown` via `truncate_left`, indeterminate `Gauge`, `rate = files/elapsed`). Polls `event::poll(80ms)` → only `q` quits during scan (Esc does NOT quit anywhere).
- `run_loop`: `terminal.draw(|f| draw(f,app))` each tick; if `pending_delete==Confirm`, modal has priority — `y/Y` confirm, `Esc/n/N/q` cancel. Otherwise: `Ctrl+C` copies (not quit), `q` quit, `Esc/Backspace/Left` go up, `Enter/Right/l` enter, `j/k` or `Up/Down`, `g`/`G` home/end, `s` toggle sort, `c/y` copy, `d/Delete` delete, `r` rescan, `h/?` help.
- **Copy**: `copy_to_clipboard` always sends **OSC 52** (`ESC ] 52 ; c ; <base64> BEL`) then tries `wl-copy`, `xclip -selection clipboard`, `xsel --clipboard --input`, `pbcopy`. Base64 hand-rolled (no crate). Returns method name for status.
- **Delete**: `request_delete` → `Confirm` → `confirm_delete` guards against deleting `root`/`opts.root`, calls `remove_dir_all` or `remove_file`, then `remove_selected_and_recalc` (remove child, `recalc_totals` recursively, `sort_tree`, fix `selected` bounds, pop breadcrumb if dir became empty).
- **Rendering**: `draw_header` (totals + breadcrumb + `s:sort:…`), `draw_list` (header row `[TOTAL COUNT SIZE GRAPH % name]`, per-row `comma/compact/human_bytes`, bar `[####      ]` 10 chars scaled to max primary, `pct_parent`), `draw_footer` (status or `1/20  ↑/↓ …`), `draw_confirm_modal` (centered_rect, Clear, red border).

### 3.6 CLI / Main

- `cli.rs`: `--hidden`/`--no-hidden` `conflicts_with` (hidden defaults **on**), `-x/--one-filesystem`/`-X/--cross-filesystem` `conflicts_with` (one_filesystem defaults **on**), `--sort count|size` `ValueEnum`, `--no-tui --top`, `--uninstall`.
- `main.rs`: resolves `include_hidden = !no_hidden`, `one_filesystem = !cross_filesystem`, `SortBy`. Early return `do_uninstall()` if `--uninstall`. `root` canonicalized via `current_dir().join(path)`. `root.exists()` check. `--no-tui` with `atty_stderr()` prints live progress to stderr (`\r` overwrite) and `print_one_shot` to stdout (pipable). `atty_like()` (stdout is_terminal) decides TUI vs fallback one-shot. Uninstall dedups candidates: `$HNFC_INSTALL_DIR/hnfc` + `~/.local/bin/hnfc` + `current_exe`; `home.clone()` needed so second `#[cfg(windows)]` push can reuse `home` (prior bug `E0382`).

## 4. Conventions & Gotchas for Future Agents

1. **Pure Docker** — don't `rustup` on host. Edit `Dockerfile`/`Makefile` if you need a new dep, not the host.
2. **Windows build must stay green** — gate every `unix`-only API with `#[cfg(unix)]` / `#[cfg(not(unix))]`. CI matrix is `ubuntu (x86_64/aarch64 musl via zig)`, `macos (x86_64/aarch64)`, `windows (x86_64 msvc)`. Test locally with the `cargo check --target x86_64-pc-windows-msvc` one-liner in §2.
3. **Don't touch `q` vs `Esc`** — spec says only `q` quits; `Esc` goes up. Don't regress.
4. **Holdover bugs**: `scripts/release.sh` originally used `gsub(/.*"/` awk that broke on `version = "0.1.0"`; fixed to `awk -F'"'`. `main.rs` `home` move required `.clone()`. Keep these fixes.
5. **Version bump** only via `scripts/release.sh` — manual `Cargo.toml` edit will miss `Cargo.lock` and git tag.
6. **Performance**: scanner is I/O bound; keep `Duration::from_millis(60)` throttle. Don't add `rayon` without measuring.
7. **Security**: TUI never follows symlinks; delete refuses to remove root. Keep it.
8. **Install names must match CI**: `hnfc-linux-amd64.tar.gz`, `hnfc-linux-arm64.tar.gz`, `hnfc-darwin-*.tar.gz`, `hnfc-windows-amd64.zip`. `install.sh` derives `ASSET` from these.

## 5. Repo Layout

```
. / target (gitignored), dist (gitignored)
Cargo.toml / Cargo.lock
Dockerfile, Makefile, .dockerignore, .gitignore
src/{cli,scan,fmt,output,tui,main}.rs
.github/workflows/release.yml
scripts/release.sh
install.sh                      # curl|sh installer
README.md                       # user-facing
AGENTS.md                       # this file (agent-facing)
LICENSE
```

## 6. Common Tasks (recipes for agents)

```sh
# check / test after any edit
make check && make test

# verify Windows doesn't break
docker run --rm -w /work -v "$PWD":/work -v hnfc-cargo-registry:/usr/local/cargo/registry --entrypoint bash hnfc-dev -lc 'export PATH=/usr/local/cargo/bin:$PATH; rustup target add x86_64-pc-windows-msvc 2>/dev/null; cargo check --target x86_64-pc-windows-msvc'

# try the binary
make build-release && ./target/release/hnfc --help
make scan ARGS="--sort size /tmp"
# TUI (needs tty): make run  or  ./target/release/hnfc

# dry-run a release
./scripts/release.sh 0.3.0 --dry-run --allow-dirty   # shows diff without committing

# actually release (bumps Cargo.toml, commits, tags, pushes -> CI publishes)
./scripts/release.sh 0.3.0
```

## 7. Release / Install Pipeline

- **Trigger**: `git tag v*` push → `.github/workflows/release.yml` (also `workflow_dispatch`).
- **Jobs**: matrix builds (see §3.6), `strip` on unix, pack to `*.tar.gz`/`*.zip` + `.sha256`, `softprops/action-gh-release@v2` publishes.
- **Install**: `install.sh` resolves `latest` via GitHub API unless `HNFC_VERSION` set, detects `OS/ARCH`, downloads `https://github.com/hmdlohar/nested-file-counter/releases/download/<tag>/<asset>`, extracts, `install -m 755` to `~/.local/bin` (or `$HNFC_INSTALL_DIR`). Uninstall via `hnfc --uninstall` or `install.sh --uninstall`.
- **Uninstall dual path**: shell (`install.sh:27`) and Rust (`main.rs:120 do_uninstall`) both support it.

## 8. Known Limitations / Future Ideas

- Scan is single-threaded (correctness over throughput). Parallel walk (`jwalk`/`ignore`) was removed to keep Windows port simple and binary small.
- TUI uses `ratatui 0.29` + `crossterm 0.28`; no `tokio`.
- No `is_pseudo_fs` on Windows — harmless because there are no `/proc` paths, but `opts.one_filesystem` is still the fast default there.
- Future: `~/.config/hnfc` ignore file, JSON output, `--json` for scripting.

## 9. When You Are the Agent

- Re-read this file + `Makefile` + `src/scan.rs` header before planning.
- Make small, verifiable edits; run `make check` after each logical chunk.
- If you add a dependency, update `Cargo.toml` and verify size (`ls -lh target/release/hnfc` — should stay ~1.5–1.7 MB).
- If you change CLI flags, update `cli.rs`, `main.rs`, `README.md` Flags table, and `AGENTS.md` §3.6.
- Leave `install.sh` asset names and `release.yml` matrix in sync.
