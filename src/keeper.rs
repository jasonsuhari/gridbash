use std::{
    cmp::Reverse,
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc,
    thread,
    time::{Duration, SystemTime},
};

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
};
use serde::Deserialize;
use serde_json::Value;

use crate::{
    app::GridPalette,
    park_watch::age_label,
    ui::{INK, SURFACE, TEXT, TEXT_DIM, TEXT_FAINT},
};

const REFRESH: Duration = Duration::from_secs(60);
const BANNER_ROWS: u16 = 2;
const FIRST_ROW: u16 = BANNER_ROWS + 3;
const ROW_STEP: u16 = 2;
const FAILING: Color = Color::Rgb(230, 92, 104);
const PASSING: Color = Color::Rgb(150, 190, 160);
const BEHIND: Color = Color::Rgb(214, 170, 120);

#[derive(Debug, Clone, Default)]
pub struct Checks {
    pub failing: usize,
    pub running: usize,
    pub passing: usize,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct BoardNote {
    #[serde(default)]
    pub doing: Option<String>,
    #[serde(default)]
    pub needs_you: Option<String>,
    #[serde(default)]
    pub tested: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PrRow {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub draft: bool,
    pub behind: Option<u64>,
    pub checks: Checks,
    pub open_threads: usize,
    pub note: BoardNote,
}

#[derive(Debug, Clone)]
pub struct KeeperSnapshot {
    pub repo: String,
    pub rows: Vec<PrRow>,
    pub checked_at: SystemTime,
    pub board_written: Option<SystemTime>,
    pub error: Option<String>,
}

impl KeeperSnapshot {
    pub fn needs_you(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| row.note.needs_you.is_some())
            .count()
    }
}

#[derive(Deserialize)]
struct Board {
    #[serde(default)]
    prs: BTreeMap<String, BoardNote>,
}

pub fn watch(cwd: PathBuf, board: Option<PathBuf>, results: mpsc::Sender<KeeperSnapshot>) {
    thread::spawn(move || {
        loop {
            if results.send(fetch(&cwd, board.as_deref())).is_err() {
                return;
            }
            thread::sleep(REFRESH);
        }
    });
}

fn fetch(cwd: &Path, board: Option<&Path>) -> KeeperSnapshot {
    let (notes, board_written) = board.map(read_board).unwrap_or_default();
    let mut snapshot = KeeperSnapshot {
        repo: String::new(),
        rows: Vec::new(),
        checked_at: SystemTime::now(),
        board_written,
        error: None,
    };
    match fetch_rows(cwd, &notes) {
        Ok((repo, rows)) => {
            snapshot.repo = repo;
            snapshot.rows = rows;
        }
        Err(error) => snapshot.error = Some(error),
    }
    snapshot
}

fn read_board(path: &Path) -> (BTreeMap<String, BoardNote>, Option<SystemTime>) {
    let written = fs::metadata(path).and_then(|meta| meta.modified()).ok();
    let notes = fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Board>(&raw).ok())
        .map(|board| board.prs)
        .unwrap_or_default();
    (notes, written)
}

const PULL_REQUESTS: &str = "query($q: String!) { search(query: $q, type: ISSUE, first: 50) { nodes { ... on PullRequest { number title url isDraft baseRefName headRefName reviewThreads(first: 100) { nodes { isResolved } } commits(last: 1) { nodes { commit { statusCheckRollup { contexts(first: 100) { nodes { __typename ... on CheckRun { status conclusion } ... on StatusContext { state } } } } } } } } } } }";

fn fetch_rows(
    cwd: &Path,
    notes: &BTreeMap<String, BoardNote>,
) -> Result<(String, Vec<PrRow>), String> {
    let repo = gh(
        cwd,
        &[
            "repo",
            "view",
            "--json",
            "nameWithOwner",
            "--jq",
            ".nameWithOwner",
        ],
    )?
    .trim()
    .to_string();
    let search = format!("repo:{repo} is:pr is:open author:@me");
    let raw = gh(
        cwd,
        &[
            "api",
            "graphql",
            "-f",
            &format!("query={PULL_REQUESTS}"),
            "-f",
            &format!("q={search}"),
        ],
    )?;
    let reply: Value = serde_json::from_str(&raw).map_err(|error| error.to_string())?;
    let nodes = reply["data"]["search"]["nodes"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let behind = behind_counts(cwd, &repo, &nodes);
    let mut rows = nodes
        .iter()
        .filter_map(|node| {
            let number = node["number"].as_u64()?;
            Some(PrRow {
                number,
                title: node["title"].as_str().unwrap_or_default().to_string(),
                url: node["url"].as_str().unwrap_or_default().to_string(),
                draft: node["isDraft"].as_bool().unwrap_or(false),
                behind: behind.get(&number).copied(),
                checks: checks_of(node),
                open_threads: node["reviewThreads"]["nodes"]
                    .as_array()
                    .map_or(0, |threads| {
                        threads
                            .iter()
                            .filter(|thread| thread["isResolved"] == Value::Bool(false))
                            .count()
                    }),
                note: notes.get(&number.to_string()).cloned().unwrap_or_default(),
            })
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| {
        (
            row.note.needs_you.is_none(),
            row.checks.failing == 0,
            row.checks.running == 0,
            Reverse(row.number),
        )
    });
    Ok((repo, rows))
}

fn checks_of(node: &Value) -> Checks {
    let mut checks = Checks::default();
    let contexts = node["commits"]["nodes"][0]["commit"]["statusCheckRollup"]["contexts"]["nodes"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    for context in contexts {
        let outcome = if context["__typename"] == "CheckRun" {
            if context["status"] != "COMPLETED" {
                "running"
            } else {
                match context["conclusion"].as_str() {
                    Some("SUCCESS" | "NEUTRAL" | "SKIPPED") => "passing",
                    _ => "failing",
                }
            }
        } else {
            match context["state"].as_str() {
                Some("SUCCESS") => "passing",
                Some("PENDING" | "EXPECTED") => "running",
                _ => "failing",
            }
        };
        match outcome {
            "running" => checks.running += 1,
            "passing" => checks.passing += 1,
            _ => checks.failing += 1,
        }
    }
    checks
}

fn behind_counts(cwd: &Path, repo: &str, nodes: &[Value]) -> BTreeMap<u64, u64> {
    let Some((owner, name)) = repo.split_once('/') else {
        return BTreeMap::new();
    };
    let quote = |text: &str| serde_json::to_string(text).unwrap_or_default();
    let fields = nodes
        .iter()
        .filter_map(|node| {
            let number = node["number"].as_u64()?;
            let base = node["baseRefName"].as_str()?;
            let head = node["headRefName"].as_str()?;
            Some(format!(
                "b{number}: ref(qualifiedName: {}) {{ compare(headRef: {}) {{ behindBy }} }}",
                quote(&format!("refs/heads/{base}")),
                quote(&format!("refs/heads/{head}")),
            ))
        })
        .collect::<Vec<_>>();
    if fields.is_empty() {
        return BTreeMap::new();
    }
    let query = format!(
        "{{ repository(owner: {}, name: {}) {{ {} }} }}",
        quote(owner),
        quote(name),
        fields.join(" ")
    );
    let Ok(raw) = gh(cwd, &["api", "graphql", "-f", &format!("query={query}")]) else {
        return BTreeMap::new();
    };
    let reply: Value = serde_json::from_str(&raw).unwrap_or_default();
    reply["data"]["repository"]
        .as_object()
        .map(|refs| {
            refs.iter()
                .filter_map(|(alias, value)| {
                    let number = alias.strip_prefix('b')?.parse().ok()?;
                    Some((number, value["compare"]["behindBy"].as_u64()?))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn gh(cwd: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("gh")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|error| format!("could not run gh: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(stderr
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("gh failed")
            .to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub struct KeeperAreas {
    pub table: Rect,
    pub pane: Rect,
}

pub fn areas(area: Rect) -> KeeperAreas {
    let table_width = (area.width * 3 / 5).max(area.width.min(60));
    KeeperAreas {
        table: Rect {
            width: table_width,
            ..area
        },
        pane: Rect {
            x: area.x + table_width,
            width: area.width - table_width,
            ..area
        },
    }
}

pub enum RowHit {
    Number(usize),
    Row(usize),
}

pub fn row_at(table: Rect, rows: usize, column: u16, row: u16) -> Option<RowHit> {
    let offset = row.checked_sub(table.y + FIRST_ROW)?;
    if offset % ROW_STEP != 0 || column < table.x || column >= table.right() {
        return None;
    }
    let index = usize::from(offset / ROW_STEP);
    if index >= rows || row >= table.bottom().saturating_sub(2) {
        return None;
    }
    Some(if column < table.x + 1 + NUMBER_WIDTH {
        RowHit::Number(index)
    } else {
        RowHit::Row(index)
    })
}

const NUMBER_WIDTH: u16 = 7;
const COLUMNS: [(&str, u16); 5] = [
    ("BEHIND", 11),
    ("CHECKS", 12),
    ("THREADS", 9),
    ("TESTED", 9),
    ("KEEPER", 0),
];

pub fn render(
    frame: &mut Frame<'_>,
    table: Rect,
    command: &str,
    snapshot: Option<&KeeperSnapshot>,
    palette: &GridPalette,
) {
    frame.render_widget(Block::default().style(Style::default().bg(INK)), table);
    let width = usize::from(table.width);
    let banner = Rect {
        height: BANNER_ROWS,
        ..table
    };
    frame.render_widget(Block::default().style(Style::default().bg(SURFACE)), banner);

    let checked = snapshot.map_or_else(
        || "checking GitHub".to_string(),
        |snapshot| {
            let notes = snapshot
                .board_written
                .map(|at| format!("notes {}   ", age_label(at)))
                .unwrap_or_default();
            format!("{notes}checked {}", age_label(snapshot.checked_at))
        },
    );
    let title = format!(
        " Keeper   > {command}{}",
        snapshot
            .filter(|snapshot| !snapshot.repo.is_empty())
            .map(|snapshot| format!(" on {}", snapshot.repo))
            .unwrap_or_default()
    );
    let gap = width.saturating_sub(title.chars().count() + checked.chars().count() + 1);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                title,
                Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::raw(" ".repeat(gap)),
            Span::styled(checked, Style::default().fg(TEXT_FAINT)),
        ])),
        Rect {
            height: 1,
            ..banner
        },
    );

    let Some(snapshot) = snapshot else {
        return;
    };
    let rows = &snapshot.rows;
    let count = |test: fn(&PrRow) -> bool| rows.iter().filter(|row| test(row)).count();
    let needs = snapshot.needs_you();
    let mut summary = vec![Span::styled(
        format!(" {} open   ", rows.len()),
        Style::default().fg(TEXT_DIM),
    )];
    if needs > 0 {
        summary.push(Span::styled(
            format!("! {needs} need you   "),
            Style::default().fg(FAILING).add_modifier(Modifier::BOLD),
        ));
    }
    summary.push(Span::styled(
        format!(
            "{} failing   {} running   {} passing",
            count(|row| row.checks.failing > 0),
            count(|row| row.checks.failing == 0 && row.checks.running > 0),
            count(|row| row.checks.failing == 0 && row.checks.running == 0),
        ),
        Style::default().fg(TEXT_DIM),
    ));
    frame.render_widget(
        Paragraph::new(Line::from(summary)),
        Rect {
            y: banner.y + 1,
            height: 1,
            ..banner
        },
    );

    if let Some(error) = snapshot.error.as_deref() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(" GitHub: {error}"),
                Style::default().fg(FAILING),
            )),
            Rect {
                y: table.y + FIRST_ROW,
                height: 1,
                ..table
            },
        );
        return;
    }

    let fixed: u16 = COLUMNS.iter().map(|(_, w)| w).sum();
    let keeper_width = (table.width / 4).max(20);
    let title_width = table
        .width
        .saturating_sub(1 + NUMBER_WIDTH + fixed + keeper_width + 1);
    let header_y = table.y + FIRST_ROW - 2;
    let mut header = vec![
        Span::raw(" "),
        cell("PR", NUMBER_WIDTH),
        cell("", title_width),
    ];
    header.extend(
        COLUMNS
            .iter()
            .map(|(name, w)| cell(name, (*w).max(len_u16(name)))),
    );
    frame.render_widget(
        Paragraph::new(Line::from(header))
            .style(Style::default().fg(TEXT_FAINT).add_modifier(Modifier::BOLD)),
        Rect {
            y: header_y,
            height: 1,
            ..table
        },
    );

    for (index, pr) in rows.iter().enumerate() {
        let y = table.y + FIRST_ROW + u16::try_from(index).unwrap_or(u16::MAX) * ROW_STEP;
        if y >= table.bottom().saturating_sub(2) {
            break;
        }
        let needs_you = pr.note.needs_you.as_deref();
        let background = if needs_you.is_some() {
            Color::Rgb(36, 16, 20)
        } else if index % 2 == 1 {
            Color::Rgb(13, 17, 22)
        } else {
            INK
        };
        let base = Style::default().bg(background);
        let (checks_text, checks_style) = if pr.checks.failing > 0 {
            (
                format!(" {} fail ", pr.checks.failing),
                Style::default()
                    .fg(TEXT)
                    .bg(palette.accent())
                    .add_modifier(Modifier::BOLD),
            )
        } else if pr.checks.running > 0 {
            (format!("{} running", pr.checks.running), base.fg(TEXT_DIM))
        } else if pr.checks.passing > 0 {
            ("pass".to_string(), base.fg(PASSING))
        } else {
            ("none".to_string(), base.fg(TEXT_FAINT))
        };
        let (behind_text, behind_style) = match pr.behind {
            Some(0) => ("fresh".to_string(), base.fg(TEXT_DIM)),
            Some(count) => (format!("{count} behind"), base.fg(BEHIND)),
            None => ("—".to_string(), base.fg(TEXT_FAINT)),
        };
        let threads = match pr.open_threads {
            0 => ("clean".to_string(), base.fg(TEXT_DIM)),
            open => (format!("{open} open"), base.fg(FAILING)),
        };
        let tested = pr.note.tested.as_deref().unwrap_or("—");
        let tested_style = match tested {
            "pass" | "passed" => base.fg(PASSING),
            "fail" | "failed" => base.fg(FAILING),
            _ => base.fg(TEXT_FAINT),
        };
        let keeper = match needs_you {
            Some(question) => Span::styled(
                truncate(&format!("! {question}"), usize::from(keeper_width)),
                base.fg(FAILING).add_modifier(Modifier::BOLD),
            ),
            None => Span::styled(
                truncate(
                    pr.note.doing.as_deref().unwrap_or(""),
                    usize::from(keeper_width),
                ),
                base.fg(TEXT),
            ),
        };
        let title = if pr.draft {
            format!("{} (draft)", pr.title)
        } else {
            pr.title.clone()
        };
        let spans = vec![
            Span::styled(" ", base),
            Span::styled(
                pad(&format!("#{}", pr.number), NUMBER_WIDTH),
                base.fg(if needs_you.is_some() { FAILING } else { TEXT })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                pad(
                    &truncate(&title, usize::from(title_width.saturating_sub(2))),
                    title_width,
                ),
                base.fg(if needs_you.is_some() { TEXT } else { TEXT_DIM }),
            ),
            Span::styled(pad(&behind_text, COLUMNS[0].1), behind_style),
            Span::styled(checks_text.clone(), checks_style),
            Span::styled(
                " ".repeat(usize::from(COLUMNS[1].1).saturating_sub(checks_text.chars().count())),
                base,
            ),
            Span::styled(pad(&threads.0, COLUMNS[2].1), threads.1),
            Span::styled(pad(tested, COLUMNS[3].1), tested_style),
            keeper,
        ];
        let row = Rect {
            y,
            height: 1,
            ..table
        };
        frame.render_widget(Block::default().style(base), row);
        frame.render_widget(Paragraph::new(Line::from(spans)), row);
    }

    frame.render_widget(
        Paragraph::new(Span::styled(
            " click a row to ask Keeper about it   click the number to open it on GitHub",
            Style::default().fg(TEXT_FAINT),
        )),
        Rect {
            y: table.bottom().saturating_sub(1),
            height: 1,
            ..table
        },
    );
}

fn len_u16(text: &str) -> u16 {
    u16::try_from(text.chars().count()).unwrap_or(u16::MAX)
}

fn cell(text: &str, width: u16) -> Span<'static> {
    Span::raw(pad(text, width))
}

fn pad(text: &str, width: u16) -> String {
    let width = usize::from(width);
    let mut out = truncate(text, width);
    let len = out.chars().count();
    out.extend(std::iter::repeat_n(' ', width.saturating_sub(len)));
    out
}

fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let mut out = text
        .chars()
        .take(width.saturating_sub(1))
        .collect::<String>();
    out.push('…');
    out
}
