use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Result;
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

#[derive(Debug, Clone, Default)]
pub struct ScanProgress {
    pub dirs: u64,
    pub files: u64,
    pub bytes: u64,
    pub current: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortBy {
    #[default]
    Count,
    Size,
}

pub fn sort_children(nodes: &mut [Node], sort: SortBy) {
    match sort {
        SortBy::Count => nodes.sort_by(|a, b| b.total.cmp(&a.total)),
        SortBy::Size => nodes.sort_by(|a, b| b.total_bytes.cmp(&a.total_bytes)),
    }
}

pub fn sort_tree(node: &mut Node, sort: SortBy) {
    sort_children(&mut node.children, sort);
    for c in &mut node.children {
        sort_tree(c, sort);
    }
}

#[derive(Debug, Clone)]
pub struct ScanOptions {
    pub root: PathBuf,
    pub include_hidden: bool,
    pub exclude_patterns: Vec<String>,
    pub max_depth: Option<usize>,
    pub one_filesystem: bool,
    pub sort: SortBy,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            root: PathBuf::from("."),
            include_hidden: true,
            exclude_patterns: vec![],
            max_depth: None,
            one_filesystem: true,
            sort: SortBy::Count,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub direct: u64,
    pub total: u64,
    pub direct_bytes: u64,
    pub total_bytes: u64,
    pub children: Vec<Node>,
    pub error: Option<String>,
}

impl Node {}

fn build_globset(patterns: &[String]) -> Option<GlobSet> {
    if patterns.is_empty() {
        return None;
    }
    let mut builder = GlobSetBuilder::new();
    for pat in patterns {
        let glob = GlobBuilder::new(pat)
            .literal_separator(true)
            .case_insensitive(false)
            .build()
            .ok()?;
        builder.add(glob);
    }
    builder.build().ok()
}

fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

fn is_pseudo_fs(path: &Path) -> bool {
    // Virtual / tmpfs mounts that are noise when scanning / (even without -x)
    const PSEUDO: &[&str] = &["/proc", "/sys", "/dev", "/run", "/snap"];
    let s = path.to_string_lossy();
    // Inside Docker the host root is at /host — treat /host/proc like /proc
    let stripped: &str = if s.starts_with("/host/") {
        &s[5..]
    } else if s == "/host" {
        "/"
    } else {
        &s
    };
    for p in PSEUDO {
        if stripped == *p || stripped.starts_with(&format!("{p}/")) {
            return true;
        }
    }
    false
}

fn should_exclude(path: &Path, name: &str, set: Option<&GlobSet>) -> bool {
    let Some(set) = set else { return false; };
    if set.is_match(name) {
        return true;
    }
    if set.is_match(path) {
        return true;
    }
    // also match against path string
    if set.is_match(path.to_string_lossy().as_ref()) {
        return true;
    }
    false
}

struct Frame {
    path: PathBuf,
    name: String,
    depth: usize,
    direct: u64,
    direct_bytes: u64,
    children: Vec<Node>,
    error: Option<String>,
    entries: Vec<fs::DirEntry>,
    idx: usize,
}

pub fn scan(opts: &ScanOptions) -> Result<Node> {
    scan_with_progress(opts, |_| {})
}

pub fn scan_with_progress<F>(opts: &ScanOptions, mut on_progress: F) -> Result<Node>
where
    F: FnMut(&ScanProgress),
{
    let root = opts.root.canonicalize().unwrap_or_else(|_| opts.root.clone());
    let globset = build_globset(&opts.exclude_patterns);

    // If root is a file, return single file node
    if let Ok(meta) = fs::symlink_metadata(&root) {
        if !meta.is_dir() {
            let name = root
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| root.to_string_lossy().to_string());
            let bytes = fs::metadata(&root).map(|m| m.len()).unwrap_or(0);
            return Ok(Node {
                name,
                path: root.clone(),
                is_dir: false,
                direct: 1,
                total: 1,
                direct_bytes: bytes,
                total_bytes: bytes,
                children: vec![],
                error: None,
            });
        }
    }

    let root_dev = if opts.one_filesystem {
        fs::metadata(&root).ok().map(|m| m.dev())
    } else {
        None
    };

    let root_name = root
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| root.to_string_lossy().to_string());

    // Collect root entries
    let (entries, error) = read_entries(&root);

    let mut stack: Vec<Frame> = vec![Frame {
        path: root.clone(),
        name: root_name,
        depth: 0,
        direct: 0,
        direct_bytes: 0,
        children: vec![],
        error,
        entries,
        idx: 0,
    }];

    // Track visited canonical paths to avoid symlink cycles (best-effort)
    let mut visited: HashSet<PathBuf> = HashSet::new();
    if let Ok(c) = root.canonicalize() {
        visited.insert(c);
    }

    let mut progress = ScanProgress {
        dirs: 1,
        files: 0,
        bytes: 0,
        current: root.clone(),
    };
    let mut last_emit = Instant::now();
    let emit_every = Duration::from_millis(60);
    // emit immediately so UI shows something right away
    on_progress(&progress);

    while !stack.is_empty() {
        let top = stack.len() - 1;
        if stack[top].idx >= stack[top].entries.len() {
            let frame = stack.pop().unwrap();
            let total = frame.direct + frame.children.iter().map(|c| c.total).sum::<u64>();
            let total_bytes = frame.direct_bytes + frame.children.iter().map(|c| c.total_bytes).sum::<u64>();
            let mut children = frame.children;
            sort_children(&mut children, opts.sort);
            let node = Node {
                name: frame.name,
                path: frame.path,
                is_dir: true,
                direct: frame.direct,
                total,
                direct_bytes: frame.direct_bytes,
                total_bytes,
                children,
                error: frame.error,
            };
            if let Some(parent) = stack.last_mut() {
                parent.children.push(node);
            } else {
                return Ok(node);
            }
            continue;
        }

        // Pop next entry (increment idx, clone needed data, release borrow before any push)
        let (name, path, ft_opt): (String, PathBuf, Option<std::fs::FileType>) = {
            let frame = &mut stack[top];
            let entry = &frame.entries[frame.idx];
            frame.idx += 1;
            let n = entry.file_name().to_string_lossy().to_string();
            let p = entry.path();
            let ft = entry.file_type().ok();
            (n, p, ft)
        };

        if !opts.include_hidden && is_hidden(&name) {
            continue;
        }
        if should_exclude(&path, &name, globset.as_ref()) {
            continue;
        }

        let Some(ft) = ft_opt else {
            let b = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            stack[top].direct += 1;
            stack[top].direct_bytes += b;
            progress.files += 1;
            progress.bytes += b;
            if last_emit.elapsed() >= emit_every {
                on_progress(&progress);
                last_emit = Instant::now();
            }
            continue;
        };

        if ft.is_symlink() {
            stack[top].direct += 1;
            // symlinks counted as files but 0 bytes
            progress.files += 1;
            if last_emit.elapsed() >= emit_every {
                on_progress(&progress);
                last_emit = Instant::now();
            }
            continue;
        }

        if ft.is_dir() {
            if let Some(max) = opts.max_depth {
                if stack[top].depth + 1 > max {
                    let node = Node {
                        name: name.clone(),
                        path: path.clone(),
                        is_dir: true,
                        direct: 0,
                        total: 0,
                        direct_bytes: 0,
                        total_bytes: 0,
                        children: vec![],
                        error: Some("max depth reached".into()),
                    };
                    stack[top].children.push(node);
                    continue;
                }
            }
            if !opts.one_filesystem && is_pseudo_fs(&path) {
                let node = Node {
                    name: name.clone(),
                    path: path.clone(),
                    is_dir: true,
                    direct: 0,
                    total: 0,
                    direct_bytes: 0,
                    total_bytes: 0,
                    children: vec![],
                    error: Some("skipped (virtual fs)".into()),
                };
                stack[top].children.push(node);
                continue;
            }
            if let Some(rd) = root_dev {
                if let Ok(meta) = fs::metadata(&path) {
                    if meta.dev() != rd {
                        let node = Node {
                            name: name.clone(),
                            path: path.clone(),
                            is_dir: true,
                            direct: 0,
                            total: 0,
                            direct_bytes: 0,
                            total_bytes: 0,
                            children: vec![],
                            error: Some("different filesystem".into()),
                        };
                        stack[top].children.push(node);
                        continue;
                    }
                }
            }
            let canon = path.canonicalize().unwrap_or_else(|_| path.clone());
            if !visited.insert(canon) {
                let node = Node {
                    name: name.clone(),
                    path: path.clone(),
                    is_dir: true,
                    direct: 0,
                    total: 0,
                    direct_bytes: 0,
                    total_bytes: 0,
                    children: vec![],
                    error: Some("cycle detected".into()),
                };
                stack[top].children.push(node);
                continue;
            }

            let parent_depth = stack[top].depth;
            let (entries, error) = read_entries(&path);
            progress.dirs += 1;
            progress.current = path.clone();
            if last_emit.elapsed() >= emit_every {
                on_progress(&progress);
                last_emit = Instant::now();
            }
            stack.push(Frame {
                path,
                name,
                depth: parent_depth + 1,
                direct: 0,
                direct_bytes: 0,
                children: vec![],
                error,
                entries,
                idx: 0,
            });
        } else {
            let b = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            stack[top].direct += 1;
            stack[top].direct_bytes += b;
            progress.files += 1;
            progress.bytes += b;
            if last_emit.elapsed() >= emit_every {
                on_progress(&progress);
                last_emit = Instant::now();
            }
        }
    }

    unreachable!("stack should have returned root node")
}

fn read_entries(path: &Path) -> (Vec<fs::DirEntry>, Option<String>) {
    match fs::read_dir(path) {
        Ok(rd) => {
            let mut v: Vec<fs::DirEntry> = Vec::new();
            let mut err: Option<String> = None;
            for e in rd {
                match e {
                    Ok(ent) => v.push(ent),
                    Err(e) => {
                        err = Some(e.to_string());
                    }
                }
            }
            (v, err)
        }
        Err(e) => (vec![], Some(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs as stdfs;
    use tempfile::TempDir;

    fn opts(root: &Path) -> ScanOptions {
        ScanOptions {
            root: root.to_path_buf(),
            include_hidden: true,
            exclude_patterns: vec![],
            max_depth: None,
            one_filesystem: true,
            sort: Default::default(),
        }
    }

    #[test]
    fn counts_nested() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        stdfs::create_dir(root.join("a")).unwrap();
        stdfs::create_dir(root.join("a").join("b")).unwrap();
        stdfs::write(root.join("a").join("b").join("f1"), "").unwrap();
        stdfs::write(root.join("a").join("f2"), "").unwrap();
        stdfs::write(root.join("f3"), "").unwrap();

        let node = scan(&opts(root)).unwrap();
        // root total = 3
        assert_eq!(node.total, 3);
        // root direct = 1 (f3)
        assert_eq!(node.direct, 1);
        let a = node.children.iter().find(|c| c.name == "a").unwrap();
        assert_eq!(a.total, 2);
        assert_eq!(a.direct, 1);
    }

    #[test]
    fn hidden_included_by_default() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        stdfs::write(root.join(".hidden"), "").unwrap();
        stdfs::write(root.join("visible"), "").unwrap();
        let node = scan(&opts(root)).unwrap();
        assert_eq!(node.total, 2);
        let mut o = opts(root);
        o.include_hidden = false;
        let node2 = scan(&o).unwrap();
        assert_eq!(node2.total, 1);
    }

    #[test]
    fn exclude_glob() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        stdfs::write(root.join("keep.txt"), "").unwrap();
        stdfs::write(root.join("skip.log"), "").unwrap();
        let mut o = opts(root);
        o.exclude_patterns = vec!["*.log".into()];
        let node = scan(&o).unwrap();
        assert_eq!(node.total, 1);
    }

    #[test]
    fn max_depth() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        stdfs::create_dir(root.join("a")).unwrap();
        stdfs::create_dir(root.join("a").join("b")).unwrap();
        stdfs::write(root.join("a").join("b").join("deep"), "").unwrap();
        let mut o = opts(root);
        o.max_depth = Some(1);
        let node = scan(&o).unwrap();
        // deep file beyond depth should not be counted
        assert_eq!(node.total, 0);
    }
}
