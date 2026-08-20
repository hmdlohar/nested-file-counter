use crate::fmt::{comma, compact, human_bytes};
use crate::scan::{Node, SortBy};

fn collect_dirs<'a>(node: &'a Node, out: &mut Vec<&'a Node>) {
    if node.is_dir {
        out.push(node);
        for c in &node.children {
            if c.is_dir {
                collect_dirs(c, out);
            }
        }
    }
}

pub fn print_one_shot(root: &Node, top: usize, sort: SortBy) {
    let mut dirs: Vec<&Node> = Vec::new();
    collect_dirs(root, &mut dirs);
    match sort {
        SortBy::Count => dirs.sort_by(|a, b| b.total.cmp(&a.total)),
        SortBy::Size => dirs.sort_by(|a, b| b.total_bytes.cmp(&a.total_bytes)),
    }

    let show = if top == 0 { dirs.len() } else { top.min(dirs.len()) };
    let max_count = dirs.first().map(|n| n.total).unwrap_or(1).max(1);

    // pre-format for width calculation (commas add width)
    let total_strs: Vec<String> = dirs.iter().map(|n| comma(n.total)).collect();
    let compact_strs: Vec<String> = dirs.iter().map(|n| compact(n.total)).collect();
    let size_strs: Vec<String> = dirs.iter().map(|n| human_bytes(n.total_bytes)).collect();
    let direct_strs: Vec<String> = dirs.iter().map(|n| comma(n.direct)).collect();

    let total_w = total_strs.iter().map(|s| s.len()).max().unwrap_or(5).max(5);
    let compact_w = compact_strs.iter().map(|s| s.len()).max().unwrap_or(4).max(4);
    let size_w = size_strs.iter().map(|s| s.len()).max().unwrap_or(4).max(7);
    let direct_w = direct_strs.iter().map(|s| s.len()).max().unwrap_or(1).max(6);

    println!(
        "{:>total_w$}  {:>compact_w$}  {:>size_w$}  {:>direct_w$}  {:<6}  {}",
        "TOTAL", "COUNT", "SIZE", "DIRECT", "GRAPH", "PATH"
    );
    println!("{}", "-".repeat(96));

    for (i, node) in dirs.iter().take(show).enumerate() {
        let bar_len = ((node.total as f64 / max_count as f64) * 10.0).round() as usize;
        let bar = format!("[{}{}]", "#".repeat(bar_len), " ".repeat(10 - bar_len));
        let err = node.error.as_deref().unwrap_or("");
        let err_suffix = if err.is_empty() { String::new() } else { format!("  (! {})", err) };
        println!(
            "{:>total_w$}  {:>compact_w$}  {:>size_w$}  {:>direct_w$}  {}  {}{}",
            total_strs[i], compact_strs[i], size_strs[i], direct_strs[i], bar, node.path.display(), err_suffix
        );
    }

    if show < dirs.len() {
        println!("\n... and {} more directories (use --top 0 for all)", dirs.len() - show);
    }
    println!(
        "\nRoot: {}  —  {} files total ({}, {})  —  {} total size ({} direct)",
        root.path.display(),
        comma(root.total),
        compact(root.total),
        comma(root.direct),
        human_bytes(root.total_bytes),
        human_bytes(root.direct_bytes),
    );
}
