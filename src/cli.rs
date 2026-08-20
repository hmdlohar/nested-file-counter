use std::path::PathBuf;

use clap::{Parser, ValueEnum};

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortArg {
    Count,
    Size,
}

#[derive(Parser, Debug, Clone)]
#[command(
    name = "hnfc",
    version,
    about = "hnfc — ncdu-like file count explorer (pure Docker build)",
    long_about = "Recursively counts files per directory and shows an ncdu-like TUI or a sorted one-shot listing.\n\nExamples:\n  hnfc                  # TUI of current dir\n  hnfc --sort size      # sort by disk size\n  hnfc /var/log         # TUI of path\n  hnfc --no-tui --top 30\n  hnfc --hidden --exclude '*.log' --exclude 'node_modules'"
)]
pub struct Cli {
    /// Path to scan (default: current directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Include hidden files and directories (dotfiles) [default: on]
    #[arg(long, conflicts_with = "no_hidden")]
    pub hidden: bool,

    /// Exclude hidden files and directories
    #[arg(long = "no-hidden", conflicts_with = "hidden")]
    pub no_hidden: bool,

    /// Exclude pattern (glob, matched against name and full path). Repeatable.
    #[arg(long = "exclude", short = 'e', value_name = "GLOB")]
    pub exclude: Vec<String>,

    /// Limit recursion depth (0 = only the root directory)
    #[arg(long, value_name = "N")]
    pub depth: Option<usize>,

    /// Stay on the same filesystem (like ncdu -x / du -x) [default: on]
    #[arg(long = "one-filesystem", short = 'x', conflicts_with = "cross_filesystem")]
    pub one_filesystem: bool,

    /// Include mounted/other filesystems (disables -x; default is to stay on one filesystem)
    #[arg(long = "cross-filesystem", short = 'X', conflicts_with = "one_filesystem")]
    pub cross_filesystem: bool,

    /// Sort by file count (default) or disk size
    #[arg(long, value_enum, default_value_t = SortArg::Count)]
    pub sort: SortArg,

    /// Print a sorted listing and exit (no TUI)
    #[arg(long)]
    pub no_tui: bool,

    /// How many entries to show in --no-tui mode (0 = all)
    #[arg(long, default_value_t = 20)]
    pub top: usize,

    /// Uninstall hnfc (removes the installed binary at $HNFC_INSTALL_DIR or ~/.local/bin)
    #[arg(long)]
    pub uninstall: bool,
}
