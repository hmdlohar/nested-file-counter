pub fn comma(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    let mut count = 0;
    for ch in s.chars().rev() {
        if count != 0 && count % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
        count += 1;
    }
    out.chars().rev().collect()
}

pub fn compact(n: u64) -> String {
    let f = n as f64;
    if n < 1_000 {
        n.to_string()
    } else if n < 1_000_000 {
        let v = f / 1_000.0;
        if v < 10.0 { format!("{:.1}k", v) } else { format!("{:.0}k", v) }
    } else if n < 1_000_000_000 {
        let v = f / 1_000_000.0;
        if v < 10.0 { format!("{:.1}M", v) } else { format!("{:.0}M", v) }
    } else {
        let v = f / 1_000_000_000.0;
        if v < 10.0 { format!("{:.1}B", v) } else { format!("{:.0}B", v) }
    }
}

pub fn human_bytes(b: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    const TB: f64 = GB * 1024.0;
    let f = b as f64;
    if b < 1024 {
        format!("{} B", b)
    } else if f < MB {
        format!("{:.1} KB", f / KB)
    } else if f < GB {
        format!("{:.1} MB", f / MB)
    } else if f < TB {
        format!("{:.1} GB", f / GB)
    } else {
        format!("{:.1} TB", f / TB)
    }
}

pub fn truncate_left(s: &str, max_chars: usize) -> String {
    let len = s.chars().count();
    if len <= max_chars || max_chars == 0 {
        return s.to_string();
    }
    if max_chars == 1 {
        return "…".to_string();
    }
    let tail: String = s.chars().skip(len - (max_chars - 1)).collect();
    format!("…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn commas() {
        assert_eq!(comma(0), "0");
        assert_eq!(comma(12), "12");
        assert_eq!(comma(1234), "1,234");
        assert_eq!(comma(1234567), "1,234,567");
        assert_eq!(comma(1000000), "1,000,000");
    }
    #[test]
    fn compacts() {
        assert_eq!(compact(999), "999");
        assert_eq!(compact(1_000), "1.0k");
        assert_eq!(compact(12_345), "12k");
        assert_eq!(compact(1_234_567), "1.2M");
        assert_eq!(compact(1_000_000_000), "1.0B");
    }
    #[test]
    fn bytes() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1024), "1.0 KB");
        assert_eq!(human_bytes(1536), "1.5 KB");
        assert_eq!(human_bytes(1024*1024), "1.0 MB");
    }
}
