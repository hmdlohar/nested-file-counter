use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, List, ListItem, ListState, Paragraph},
    Frame, Terminal,
};

use crate::fmt::{comma, compact, human_bytes, truncate_left};
use crate::scan::{scan, scan_with_progress, sort_tree, Node, ScanOptions, ScanProgress, SortBy};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingDelete {
    None,
    Confirm,
}

struct App {
    root: Node,
    path: Vec<usize>,
    selected: usize,
    opts: ScanOptions,
    sort: SortBy,
    status: String,
    pending_delete: PendingDelete,
}

impl App {
    fn new(root: Node, opts: ScanOptions) -> Self {
        let sort = opts.sort;
        Self {
            root,
            path: vec![],
            selected: 0,
            opts,
            sort,
            status: String::new(),
            pending_delete: PendingDelete::None,
        }
    }

    fn set_sort(&mut self, sort: SortBy) {
        if self.sort == sort {
            return;
        }
        self.sort = sort;
        sort_tree(&mut self.root, self.sort);
        self.selected = 0;
        self.status = format!(
            "Sorted by {}",
            match sort {
                SortBy::Count => "count",
                SortBy::Size => "size",
            }
        );
    }

    fn current_node(&self) -> &Node {
        let mut n = &self.root;
        for &idx in &self.path {
            n = &n.children[idx];
        }
        n
    }

    fn selected_node(&self) -> Option<&Node> {
        let cur = self.current_node();
        cur.children.get(self.selected)
    }

    fn selected_path(&self) -> Option<PathBuf> {
        self.selected_node().map(|n| n.path.clone())
    }

    fn copy_selected_path(&mut self) -> Result<()> {
        let Some(path) = self.selected_path() else {
            self.status = "Nothing selected to copy".into();
            return Ok(());
        };
        let s = path.display().to_string();
        let msg = match copy_to_clipboard(&s) {
            Ok(method) => format!("Copied ({method}): {}", truncate_middle(&s, 60)),
            Err(e) => format!("Copy failed ({}), OSC52 sent: {}", e, truncate_middle(&s, 50)),
        };
        self.status = msg;
        Ok(())
    }

    fn request_delete(&mut self) {
        if self.selected_node().is_none() {
            self.status = "Nothing selected to delete".into();
            return;
        }
        self.pending_delete = PendingDelete::Confirm;
    }

    fn cancel_delete(&mut self) {
        self.pending_delete = PendingDelete::None;
        self.status = "Delete cancelled".into();
    }

    fn confirm_delete(&mut self) -> Result<()> {
        let Some(target) = self.selected_node().map(|n| n.path.clone()) else {
            self.pending_delete = PendingDelete::None;
            return Ok(());
        };
        let is_dir = self.selected_node().map(|n| n.is_dir).unwrap_or(false);
        // Safety: never delete root itself
        if target == self.root.path || target == self.opts.root.canonicalize().unwrap_or(self.opts.root.clone()) {
            self.status = "Refusing to delete root".into();
            self.pending_delete = PendingDelete::None;
            return Ok(());
        }

        let res = if is_dir {
            fs::remove_dir_all(&target)
        } else {
            fs::remove_file(&target)
        };
        self.pending_delete = PendingDelete::None;
        match res {
            Ok(()) => {
                self.status = format!("Deleted: {}", truncate_middle(&target.display().to_string(), 70));
                self.remove_selected_and_recalc();
            }
            Err(e) => {
                self.status = format!("Delete failed: {e}");
            }
        }
        Ok(())
    }

    fn remove_selected_and_recalc(&mut self) {
        // Remove child at path+selected, then recompute aggregates up the tree.
        let depth = self.path.len();
        // Walk to parent via mutable borrow
        fn remove_at(root: &mut Node, path: &[usize], idx: usize) {
            let mut n = root;
            for &i in path {
                n = &mut n.children[i];
            }
            if idx < n.children.len() {
                n.children.remove(idx);
            }
        }
        remove_at(&mut self.root, &self.path, self.selected);
        recalc_totals(&mut self.root);
        // Resort at every level by current sort
        sort_tree(&mut self.root, self.sort);
        // Fix selection bounds
        let cur = self.current_node();
        if cur.children.is_empty() {
            // If we just emptied the current dir, go up one level to avoid showing empty dir with stale header
            if depth > 0 {
                if let Some(idx) = self.path.pop() {
                    // keep parent selection where we left it, clamped
                    let parent_len = self.current_node().children.len();
                    if parent_len == 0 {
                        self.selected = 0;
                    } else {
                        self.selected = idx.min(parent_len - 1);
                    }
                    self.status = format!("{} — moved up (now empty)", self.status);
                }
            } else {
                self.selected = 0;
            }
        } else if self.selected >= cur.children.len() {
            self.selected = cur.children.len() - 1;
        }
    }

    fn rescan_with_progress(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
        // Re-run scan with live progress overlay, reusing scanning UI
        let (progress_tx, progress_rx) = mpsc::channel::<ScanProgress>();
        let (result_tx, result_rx) = mpsc::channel::<Result<Node, String>>();
        let opts = self.opts.clone();
        thread::spawn(move || {
            let r = scan_with_progress(&opts, |p| {
                let _ = progress_tx.send(p.clone());
            });
            let _ = result_tx.send(r.map_err(|e| e.to_string()));
        });

        let mut last = ScanProgress {
            dirs: 0,
            files: 0,
            bytes: 0,
            current: self.opts.root.clone(),
        };
        let start = Instant::now();
        let mut spinner: usize = 0;
        const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

        loop {
            while let Ok(p) = progress_rx.try_recv() {
                last = p;
            }
            if let Ok(r) = result_rx.try_recv() {
                match r {
                    Ok(new_root) => {
                        self.root = new_root;
                        let mut node = &self.root;
                        let mut valid_path = Vec::new();
                        for &idx in &self.path.clone() {
                            if idx < node.children.len() {
                                valid_path.push(idx);
                                node = &node.children[idx];
                            } else {
                                break;
                            }
                        }
                        self.path = valid_path;
                        self.selected = 0;
                        self.status.clear();
                        return Ok(());
                    }
                    Err(e) => {
                        self.status = format!("scan error: {e}");
                        return Ok(());
                    }
                }
            }

            terminal.draw(|f| draw_scanning(f, &last, start.elapsed(), SPINNER[spinner % SPINNER.len()], &self.opts, true))?;
            spinner = spinner.wrapping_add(1);

            if event::poll(Duration::from_millis(80))? {
                if let Event::Key(k) = event::read()? {
                    if k.kind == KeyEventKind::Press && k.code == KeyCode::Char('q') {
                        self.status = "Rescan cancelled (showing old data)".into();
                        return Ok(());
                    }
                }
            }
        }
    }

    fn enter_selected(&mut self) {
        let cur = self.current_node();
        if self.selected >= cur.children.len() {
            return;
        }
        let child = &cur.children[self.selected];
        if child.is_dir && !child.children.is_empty() {
            self.path.push(self.selected);
            self.selected = 0;
        } else if child.is_dir {
            self.status = "Empty directory (no file-containing subdirs shown)".into();
        }
    }

    fn go_up(&mut self) {
        if let Some(idx) = self.path.pop() {
            self.selected = idx;
            self.status.clear();
        }
    }
}

fn breadcrumb(app: &App) -> String {
    let mut parts: Vec<String> = vec![app.root.path.display().to_string()];
    let mut n = &app.root;
    for &idx in &app.path {
        n = &n.children[idx];
        parts.push(n.name.clone());
    }
    parts.join(" / ")
}

// New entry: does its own scan with live progress screen
pub fn run_tui(opts: ScanOptions) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let root = scan_with_live_progress(&mut terminal, &opts)?;

    let Some(root) = root else {
        // user quit during scan
        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        terminal.show_cursor()?;
        return Ok(());
    };

    let mut app = App::new(root, opts);
    let result = run_loop(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn scan_with_live_progress(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    opts: &ScanOptions,
) -> Result<Option<Node>> {
    // fast path: if root is a single file, scan is instant — skip progress UI
    if let Ok(meta) = std::fs::symlink_metadata(&opts.root) {
        if !meta.is_dir() {
            return Ok(Some(scan(opts)?));
        }
    }

    let (progress_tx, progress_rx) = mpsc::channel::<ScanProgress>();
    let (result_tx, result_rx) = mpsc::channel::<Result<Node, String>>();
    let opts2 = opts.clone();
    thread::spawn(move || {
        let r = scan_with_progress(&opts2, |p| {
            let _ = progress_tx.send(p.clone());
        });
        let _ = result_tx.send(r.map_err(|e| e.to_string()));
    });

    let mut last = ScanProgress {
        dirs: 0,
        files: 0,
        bytes: 0,
        current: opts.root.clone(),
    };
    let start = Instant::now();
    let mut spinner: usize = 0;
    const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

    loop {
        while let Ok(p) = progress_rx.try_recv() {
            last = p;
        }
        if let Ok(r) = result_rx.try_recv() {
            match r {
                Ok(node) => return Ok(Some(node)),
                Err(e) => anyhow::bail!("scan failed: {e}"),
            }
        }

        terminal.draw(|f| draw_scanning(f, &last, start.elapsed(), SPINNER[spinner % SPINNER.len()], opts, false))?;
        spinner = spinner.wrapping_add(1);

        if event::poll(Duration::from_millis(80))? {
            if let Event::Key(k) = event::read()? {
                if k.kind == KeyEventKind::Press && k.code == KeyCode::Char('q') {
                    return Ok(None);
                }
            }
        }
    }
}

fn draw_scanning(f: &mut Frame, p: &ScanProgress, elapsed: Duration, spinner: &str, opts: &ScanOptions, is_rescan: bool) {
    let area = f.area();
    let title = if is_rescan { " Rescanning… (q to cancel) " } else { " Scanning… (q to quit) " };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(title);

    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(4),
            Constraint::Length(2),
            Constraint::Min(1),
        ])
        .split(inner);

    // Title line with spinner
    let title_line = Paragraph::new(Line::from(vec![
        Span::styled(format!(" {} ", spinner), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::styled(
            if is_rescan { "Rescanning " } else { "Scanning " },
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::styled(opts.root.display().to_string(), Style::default().fg(Color::DarkGray)),
        Span::raw(format!("  —  {:.1}s", elapsed.as_secs_f64())),
    ]))
    .alignment(Alignment::Center);
    f.render_widget(title_line, chunks[0]);

    // Stats: dirs / files (comma + compact) / size
    let stats = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("dirs: ", Style::default().fg(Color::DarkGray)),
            Span::styled(comma(p.dirs), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("files: ", Style::default().fg(Color::DarkGray)),
            Span::styled(comma(p.files), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(format!("({})", compact(p.files)), Style::default().fg(Color::Cyan)),
            Span::raw("  "),
            Span::styled(human_bytes(p.bytes), Style::default().fg(Color::Green)),
        ]),
    ])
    .alignment(Alignment::Center)
    .block(Block::default().borders(Borders::ALL).title(" Progress ").border_style(Style::default().fg(Color::DarkGray)));
    f.render_widget(stats, chunks[1]);

    // Current path — left-truncated so the tail (where you're actually scanning) stays visible
    let cur_str = p.current.display().to_string();
    let avail = chunks[2].width.saturating_sub(10) as usize; // "current: " ~9 chars + border
    let shown = truncate_left(&cur_str, avail.max(20).min(160));
    let rate = p.files as f64 / elapsed.as_secs_f64().max(0.2);
    let rate_str = if rate >= 1000.0 {
        format!("  {} files/s", compact(rate as u64))
    } else {
        format!("  {:.0} files/s", rate)
    };
    let cur_line = Paragraph::new(Line::from(vec![
        Span::styled("current: ", Style::default().fg(Color::DarkGray)),
        Span::styled(shown, Style::default().fg(Color::White)),
        Span::styled(rate_str, Style::default().fg(Color::DarkGray)),
    ]))
    .alignment(Alignment::Left);
    f.render_widget(cur_line, chunks[2]);

    // Gauge-ish bar — indeterminate: animate fill based on spinner tick
    let pct = ((elapsed.as_millis() as f64 / 12.0) % 100.0) / 100.0;
    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title(" Working… ").border_style(Style::default().fg(Color::DarkGray)))
        .gauge_style(Style::default().fg(Color::Cyan).bg(Color::Black))
        .ratio(pct)
        .label(format!("{} files  {} dirs  {}", comma(p.files), comma(p.dirs), human_bytes(p.bytes)));
    f.render_widget(gauge, chunks[3]);
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    loop {
        terminal.draw(|f| draw(f, app))?;

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                // Confirmation modal has priority
                if app.pending_delete == PendingDelete::Confirm {
                    match key.code {
                        KeyCode::Char('y') | KeyCode::Char('Y') => app.confirm_delete()?,
                        KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Char('q') => app.cancel_delete(),
                        _ => {}
                    }
                    continue;
                }
                // Ctrl+C / Ctrl+Y also copy (common muscle memory)
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    match key.code {
                        KeyCode::Char('c') => {
                            // Ctrl+C: copy, don't quit — q is quit per spec
                            app.copy_selected_path()?;
                            continue;
                        }
                        _ => {}
                    }
                }
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Esc => app.go_up(),
                    KeyCode::Backspace | KeyCode::Left => app.go_up(),
                    KeyCode::Down | KeyCode::Char('j') => {
                        let n = app.current_node().children.len();
                        if n > 0 {
                            app.selected = (app.selected + 1).min(n - 1);
                        }
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        if app.selected > 0 {
                            app.selected -= 1;
                        }
                    }
                    KeyCode::Home | KeyCode::Char('g') => app.selected = 0,
                    KeyCode::End | KeyCode::Char('G') => {
                        let n = app.current_node().children.len();
                        if n > 0 {
                            app.selected = n - 1;
                        }
                    }
                    KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => app.enter_selected(),
                    KeyCode::Char('r') => app.rescan_with_progress(terminal)?,
                    KeyCode::Char('s') => {
                        let next = match app.sort {
                            SortBy::Count => SortBy::Size,
                            SortBy::Size => SortBy::Count,
                        };
                        app.set_sort(next);
                    }
                    KeyCode::Char('y') | KeyCode::Char('c') => app.copy_selected_path()?,
                    KeyCode::Char('d') | KeyCode::Delete => app.request_delete(),
                    KeyCode::Char('h') | KeyCode::Char('?') => {
                        app.status = "Keys: ↑/↓ j/k nav  →/Enter/l enter  ←/Esc up  s sort  c/y copy path  d/Del delete  r rescan  q quit".into();
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .split(area);

    draw_header(f, app, chunks[0]);
    draw_list(f, app, chunks[1]);
    draw_footer(f, app, chunks[2]);

    if app.pending_delete == PendingDelete::Confirm {
        draw_confirm_modal(f, app, area);
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let cur = app.current_node();
    let total_label = format!("hnfc — {} files ({})", comma(app.root.total), compact(app.root.total));
    let size_label = human_bytes(app.root.total_bytes);
    let cur_label = if app.path.is_empty() {
        format!("{}  ({} direct, {})", app.root.path.display(), comma(cur.direct), human_bytes(cur.direct_bytes))
    } else {
        format!("{}  ({} files, {} direct — {})", cur.name, comma(cur.total), comma(cur.direct), human_bytes(cur.total_bytes))
    };
    let bc = breadcrumb(app);
    let lines = vec![
        Line::from(vec![
            Span::styled(total_label, Style::default().add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(size_label, Style::default().fg(Color::Green)),
            Span::raw("   "),
            Span::styled(cur_label, Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![Span::styled(bc, Style::default().fg(Color::DarkGray))]),
    ];
    let sort_label = match app.sort {
        SortBy::Count => "sort:count",
        SortBy::Size => "sort:size",
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(format!(" hnfc — file counts (s:{sort_label}) "));
    let p = Paragraph::new(lines).block(block);
    f.render_widget(p, area);
}

fn draw_list(f: &mut Frame, app: &App, area: Rect) {
    let cur = app.current_node();
    let children = &cur.children;

    if children.is_empty() {
        let msg = if cur.is_dir {
            if cur.error.is_some() {
                format!("No subdirectories ({}).", cur.error.as_deref().unwrap_or(""))
            } else {
                "No subdirectories with files.".to_string()
            }
        } else {
            "Not a directory.".to_string()
        };
        let p = Paragraph::new(msg).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {} ", cur.path.display())),
        );
        f.render_widget(p, area);
        return;
    }

    let (max_primary, parent_primary) = match app.sort {
        SortBy::Count => (
            children.iter().map(|c| c.total).max().unwrap_or(1).max(1) as f64,
            cur.total.max(1) as f64,
        ),
        SortBy::Size => (
            children.iter().map(|c| c.total_bytes).max().unwrap_or(1).max(1) as f64,
            cur.total_bytes.max(1) as f64,
        ),
    };

    let items: Vec<ListItem> = children
        .iter()
        .enumerate()
        .map(|(idx, child)| {
            let primary = match app.sort {
                SortBy::Count => child.total as f64,
                SortBy::Size => child.total_bytes as f64,
            };
            let pct_parent = (primary / parent_primary * 100.0).round() as u64;
            let bar_len = ((primary / max_primary) * 10.0).round() as usize;
            let bar = format!("[{}{}]", "#".repeat(bar_len), " ".repeat(10 - bar_len));
            let kind = if child.is_dir { "/" } else { " " };
            let err = child
                .error
                .as_deref()
                .map(|e| format!(" (!{})", e))
                .unwrap_or_default();
            let line = Line::from(vec![
                Span::styled(format!("{:>10}", comma(child.total)), Style::default().fg(Color::Yellow)),
                Span::raw(" "),
                Span::styled(format!("{:>6}", compact(child.total)), Style::default().fg(Color::Cyan)),
                Span::raw(" "),
                Span::styled(format!("{:>9}", human_bytes(child.total_bytes)), Style::default().fg(Color::Green)),
                Span::raw(" "),
                Span::styled(bar, Style::default().fg(Color::Green)),
                Span::raw(format!(" {:>3}% ", pct_parent)),
                Span::styled(
                    format!("{}{}{}", child.name, kind, err),
                    if idx == app.selected {
                        Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    },
                ),
            ]);
            ListItem::new(line)
        })
        .collect();

    // header hint row above list — rendered as block title with column labels
    let title = format!(
        " {} — {} entries  [TOTAL  COUNT    SIZE    GRAPH  %  name] ",
        cur.path.display(),
        comma(children.len() as u64)
    );
    let mut state = ListState::default();
    state.select(Some(app.selected));

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(Style::default().fg(Color::DarkGray)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    f.render_stateful_widget(list, area, &mut state);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let cur = app.current_node();
    let hint = if !app.status.is_empty() {
        app.status.clone()
    } else if cur.children.is_empty() {
        format!(
            "q quit  r rescan  s sort({})  c/y copy  d delete  ← up",
            match app.sort {
                SortBy::Count => "count",
                SortBy::Size => "size",
            }
        )
    } else {
        format!(
            " {}/{}  ↑/↓ nav  →/Enter enter  ←/Esc up  s sort({})  c/y copy  d delete  r rescan  q quit  h help ",
            comma((app.selected + 1) as u64),
            comma(cur.children.len() as u64),
            match app.sort {
                SortBy::Count => "count",
                SortBy::Size => "size",
            }
        )
    };
    let p = Paragraph::new(hint).style(Style::default().fg(Color::DarkGray).bg(Color::Black));
    f.render_widget(p, area);
}

fn draw_confirm_modal(f: &mut Frame, app: &App, area: Rect) {
    let Some(target) = app.selected_node() else { return; };
    let path_str = target.path.display().to_string();
    let kind = if target.is_dir { "directory" } else { "file" };
    // Truncate path to fit
    let max_w = area.width.saturating_sub(10) as usize;
    let shown = truncate_middle(&path_str, max_w.max(40));
    let lines = vec![
        Line::from(Span::styled(
            "  Confirm delete  ",
            Style::default().fg(Color::White).bg(Color::Red).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::raw("Delete "),
            Span::styled(kind, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw(": "),
        ]),
        Line::from(Span::styled(shown, Style::default().fg(Color::White))),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(comma(target.total), Style::default().fg(Color::Yellow)),
            Span::raw(" files  "),
            Span::styled(human_bytes(target.total_bytes), Style::default().fg(Color::Green)),
        ]),
        Line::from(""),
        Line::from(Span::styled("  This cannot be undone.  ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(vec![
            Span::styled(" y ", Style::default().fg(Color::Black).bg(Color::Red).add_modifier(Modifier::BOLD)),
            Span::raw(" Yes, delete    "),
            Span::styled(" n/Esc ", Style::default().fg(Color::Black).bg(Color::DarkGray).add_modifier(Modifier::BOLD)),
            Span::raw(" Cancel "),
        ]),
    ];
    let h = (lines.len() as u16 + 4).min(area.height.saturating_sub(4));
    let w = (max_w as u16 + 6).min(area.width.saturating_sub(4)).max(44);
    let popup = centered_rect(w, h, area);
    // Clear behind modal
    f.render_widget(ratatui::widgets::Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .title(" Delete? ");
    let p = Paragraph::new(lines)
        .block(block)
        .alignment(Alignment::Center)
        .wrap(ratatui::widgets::Wrap { trim: false });
    f.render_widget(p, popup);
}

fn centered_rect(w: u16, h: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect { x, y, width: w, height: h }
}

fn truncate_middle(s: &str, max_chars: usize) -> String {
    let len = s.chars().count();
    if len <= max_chars {
        return s.to_string();
    }
    if max_chars <= 3 {
        return s.chars().take(max_chars).collect();
    }
    let keep = (max_chars - 3) / 2;
    let left: String = s.chars().take(keep + (max_chars % 2)).collect();
    let right: String = s.chars().skip(len - keep).collect();
    format!("{left}...{right}")
}

fn copy_to_clipboard(text: &str) -> Result<String, String> {
    // Always emit OSC52 so any terminal that supports it (tmux, kitty, wezterm, etc.) picks it up,
    // then also try native clipboard tools as a second channel.
    let _ = write_osc52(text);

    // Try wl-copy, then xclip, then xsel, then pbcopy
    let attempts: &[(&str, &[&str])] = &[
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
        ("pbcopy", &[]),
    ];
    for (bin, args) in attempts {
        if let Ok(mut child) = Command::new(bin)
            .args(*args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            if let Some(stdin) = child.stdin.as_mut() {
                if stdin.write_all(text.as_bytes()).is_ok() {
                    let _ = stdin.flush();
                }
            }
            // Don't block long — give it a moment
            match child.wait() {
                Ok(st) if st.success() => return Ok(bin.to_string()),
                _ => continue,
            }
        }
    }
    // OSC52 was still sent, so report that
    Ok("OSC52".to_string())
}

fn write_osc52(text: &str) -> io::Result<()> {
    // OSC 52 ; c ; base64 BEL — copy to clipboard via terminal
    use std::io::IsTerminal;
    if !std::io::stdout().is_terminal() {
        return Ok(());
    }
    // base64 without external crate
    let b64 = base64_encode(text.as_bytes());
    let seq = format!("\x1b]52;c;{b64}\x07");
    let mut out = io::stdout();
    out.write_all(seq.as_bytes())?;
    out.flush()
}

fn base64_encode(bytes: &[u8]) -> String {
    const ALPH: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPH[((n >> 18) & 63) as usize] as char);
        out.push(ALPH[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { ALPH[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { ALPH[(n & 63) as usize] as char } else { '=' });
    }
    out
}

fn recalc_totals(node: &mut Node) {
    if !node.is_dir {
        return;
    }
    let mut total = node.direct;
    let mut total_bytes = node.direct_bytes;
    for c in &mut node.children {
        recalc_totals(c);
        total += c.total;
        total_bytes += c.total_bytes;
    }
    node.total = total;
    node.total_bytes = total_bytes;
}
