mod cli;
mod fmt;
mod output;
mod scan;
mod tui;

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

use cli::Cli;
use scan::{scan, scan_with_progress, ScanOptions};

fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.uninstall {
        return do_uninstall();
    }

    let root = if cli.path.is_absolute() {
        cli.path.clone()
    } else {
        std::env::current_dir()
            .unwrap_or(PathBuf::from("."))
            .join(&cli.path)
    };

    let sort = match cli.sort {
        cli::SortArg::Count => scan::SortBy::Count,
        cli::SortArg::Size => scan::SortBy::Size,
    };
    // Default: stay on one filesystem (fast). --cross-filesystem / -X opts into crossing mounts.
    // --one-filesystem / -x kept as explicit alias for the default.
    let one_filesystem = if cli.cross_filesystem {
        false
    } else {
        true
    };
    // Default: include hidden + stay on one FS. --no-hidden / --cross-filesystem opt out.
    let include_hidden = if cli.no_hidden { false } else { true };
    let opts = ScanOptions {
        root,
        include_hidden,
        exclude_patterns: cli.exclude.clone(),
        max_depth: cli.depth,
        one_filesystem,
        sort,
    };

    if !opts.root.exists() {
        eprintln!("hnfc: path does not exist: {}", opts.root.display());
        std::process::exit(2);
    }

    if cli.no_tui {
        let tree = if atty_stderr() {
            // live progress on stderr so stdout stays clean for piping
            let start = std::time::Instant::now();
            let mut last_line_len = 0usize;
            let res = scan_with_progress(&opts, |p| {
                use std::io::Write;
                let line = format!(
                    "Scanning  dirs {}  files {} ({})  {}  —  {}",
                    fmt::comma(p.dirs),
                    fmt::comma(p.files),
                    fmt::compact(p.files),
                    fmt::human_bytes(p.bytes),
                    p.current.display()
                );
                let mut stderr = std::io::stderr();
                // clear previous line
                let pad = if line.len() < last_line_len {
                    " ".repeat(last_line_len - line.len())
                } else {
                    String::new()
                };
                let _ = write!(stderr, "\r{}{}", line, pad);
                let _ = stderr.flush();
                last_line_len = line.len().max(last_line_len);
                // keep compiler happy about unused start var in closure capture
                let _ = &start;
            })?;
            eprintln!("\rDone in {:.1}s — {} dirs, {} files ({})  {} total\x1b[K",
                start.elapsed().as_secs_f64(),
                fmt::comma(res.total), // dirs not trivial; use progress dirs? show root total
                fmt::comma(res.total),
                fmt::compact(res.total),
                fmt::human_bytes(res.total_bytes),
            );
            // also clear line properly
            res
        } else {
            scan(&opts)?
        };
        output::print_one_shot(&tree, cli.top, sort);
    } else {
        if !atty_like() {
            let tree = scan(&opts)?;
            output::print_one_shot(&tree, cli.top, sort);
        } else {
            tui::run_tui(opts)?;
        }
    }

    Ok(())
}

fn atty_like() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal()
}

fn atty_stderr() -> bool {
    use std::io::IsTerminal;
    std::io::stderr().is_terminal()
}

fn do_uninstall() -> Result<()> {
    // Mirrors install.sh defaults:
    // 1. $HNFC_INSTALL_DIR/hnfc if set
    // 2. ~/.local/bin/hnfc
    // 3. the currently running binary (via std::env::current_exe)
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(d) = std::env::var("HNFC_INSTALL_DIR") {
        candidates.push(PathBuf::from(d).join("hnfc"));
        #[cfg(windows)]
        candidates.push(PathBuf::from(std::env::var("HNFC_INSTALL_DIR").unwrap()).join("hnfc.exe"));
    }
    if let Ok(home) = std::env::var("HOME") {
        candidates.push(PathBuf::from(home).join(".local/bin/hnfc"));
        #[cfg(windows)]
        candidates.push(PathBuf::from(home).join(".local/bin/hnfc.exe"));
    }
    if let Ok(exe) = std::env::current_exe() {
        candidates.push(exe);
    }
    // Deduplicate
    candidates.sort();
    candidates.dedup();

    let mut removed = false;
    for p in &candidates {
        if p.exists() {
            match std::fs::remove_file(p) {
                Ok(_) => {
                    println!("Removed {}", p.display());
                    removed = true;
                }
                Err(e) => eprintln!("Could not remove {}: {e}", p.display()),
            }
        }
    }
    if removed {
        println!("hnfc uninstalled. If you added ~/.local/bin to PATH, you can leave it as-is.");
    } else {
        eprintln!("hnfc binary not found in any of:");
        for p in &candidates {
            eprintln!("  {}", p.display());
        }
        eprintln!("Nothing removed. Try: HNFC_INSTALL_DIR=/your/dir hnfc --uninstall  or  rm $(which hnfc)");
        std::process::exit(1);
    }
    Ok(())
}
