use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant, SystemTime},
};

use serde::Deserialize;

use crate::layout::PaneId;

/// A thinking Claude repaints its spinner several times a second, so a screen
/// this still means its turn is over.
pub const QUIET_BEFORE_JUDGING: Duration = Duration::from_secs(5);
const BUSY_PANE_INTERVAL: Duration = Duration::from_secs(300);
const SCREEN_LINES: usize = 60;
const JUDGE_TIMEOUT: Duration = Duration::from_secs(90);
const REVIEW_TIMEOUT: Duration = Duration::from_secs(600);
const VERDICT_SCHEMA: &str = r#"{"type":"object","properties":{"met":{"type":"boolean"},"reason":{"type":"string"}},"required":["met","reason"],"additionalProperties":false}"#;

#[derive(Debug, Clone)]
pub struct Verdict {
    pub reason: String,
    pub at: SystemTime,
}

#[derive(Debug, Clone)]
pub struct ParkWatch {
    pub condition: String,
    pub met: Option<Verdict>,
    pub last: Option<Verdict>,
    pub seen: bool,
    pub error: Option<String>,
    /// An agent pane told to run `gridbash agent done` is judged on that
    /// claim, never on its screen.
    pub reports_done: bool,
    /// Whether the pane has been told it is parked and how to report done.
    pub briefed: bool,
    judging: bool,
    screen_revision: u64,
    changed_at: Instant,
    judged_revision: Option<u64>,
    judged_at: Option<Instant>,
}

impl ParkWatch {
    pub fn new(condition: String, reports_done: bool) -> Self {
        Self {
            condition,
            met: None,
            last: None,
            seen: false,
            error: None,
            reports_done,
            briefed: false,
            judging: false,
            screen_revision: 0,
            changed_at: Instant::now(),
            judged_revision: None,
            judged_at: None,
        }
    }

    pub fn restored(
        condition: String,
        met: Option<Verdict>,
        reports_done: bool,
        briefed: bool,
    ) -> Self {
        Self {
            seen: met.is_some(),
            met,
            briefed,
            ..Self::new(condition, reports_done)
        }
    }

    pub fn needs_briefing(&self) -> bool {
        self.reports_done && !self.briefed && self.met.is_none()
    }

    pub fn judging(&self) -> bool {
        self.judging
    }

    pub fn unseen_done(&self) -> bool {
        self.met.is_some() && !self.seen
    }

    pub fn due(&mut self, revision: u64, now: Instant) -> bool {
        if self.met.is_some() || self.judging || self.reports_done {
            return false;
        }
        if revision != self.screen_revision {
            self.screen_revision = revision;
            self.changed_at = now;
        }
        if self.judged_revision == Some(revision) {
            return false;
        }
        let still = now.duration_since(self.changed_at) >= QUIET_BEFORE_JUDGING;
        let overdue = self
            .judged_at
            .is_none_or(|at| now.duration_since(at) >= BUSY_PANE_INTERVAL);
        still || overdue
    }

    pub fn start(&mut self, now: Instant) {
        self.judging = true;
        self.judged_revision = Some(self.screen_revision);
        self.judged_at = Some(now);
    }

    pub fn record_verdict(&mut self, outcome: Result<(bool, String), String>) -> bool {
        self.judging = false;
        match outcome {
            Ok((met, reason)) => {
                self.error = None;
                let verdict = Verdict {
                    reason,
                    at: SystemTime::now(),
                };
                self.last = Some(verdict.clone());
                if met {
                    self.met = Some(verdict);
                    self.seen = false;
                }
                met
            }
            Err(error) => {
                self.error = Some(error);
                false
            }
        }
    }
}

pub struct Judgment {
    pub pane: PaneId,
    pub outcome: Result<(bool, String), String>,
    pub tokens: u64,
    pub review: bool,
}

/// What a parked agent is told, so it can finish without the person and say
/// when it has.
pub fn briefing(condition: &str) -> String {
    format!(
        "You have been parked: the person moved this pane off their main grid so you can finish \
         on your own. You go back to them when this holds: {condition}. Keep working without \
         waiting for them. When you believe it holds, run `gridbash agent done \"<what you did \
         and how you checked it>\"`. A reviewer then checks your claim against the repository \
         and answers here."
    )
}

/// The reviewer's answer, typed back into the agent's pane.
pub fn review_reply(outcome: &Result<(bool, String), String>) -> String {
    match outcome {
        Ok((true, reason)) => {
            format!("The reviewer accepted your claim: {reason} The person has been told.")
        }
        Ok((false, reason)) => format!(
            "The reviewer did not accept your claim yet: {reason} Fix what it points at or \
             answer it, then run `gridbash agent done` again."
        ),
        Err(error) => format!(
            "The review could not run ({error}). Run `gridbash agent done` again in a few minutes."
        ),
    }
}

pub fn review(
    pane: PaneId,
    condition: &str,
    claim: &str,
    screen: String,
    cwd: PathBuf,
    results: mpsc::Sender<Judgment>,
) {
    let commands = READ_ONLY_COMMANDS.join(", ");
    let prompt = format!(
        "An agent working in this repository may report back to a person only once this \
         condition holds: {condition}\n\
         It now claims the condition holds:\n{claim}\n\
         The last lines of its terminal:\n{screen}\n\
         Check the claim yourself by reading files and running these read-only commands: \
         {commands}. Take nothing on its word that you can check. Reply met only if what \
         you found shows the condition holds. Otherwise reply not met, and as the reason tell \
         the agent, in at most three sentences, what is missing or wrong."
    );
    thread::spawn(move || {
        let (outcome, tokens) = match run_claude(&prompt, &cwd, &REVIEWER) {
            Ok(reply) => reply.verdict(),
            Err(error) => (Err(error), 0),
        };
        let _ = results.send(Judgment {
            pane,
            outcome,
            tokens,
            review: true,
        });
    });
}

pub fn screen_text(screen: &vt100::Screen) -> String {
    let contents = screen.contents();
    let lines = contents.lines().map(str::trim_end).collect::<Vec<_>>();
    let end = lines
        .iter()
        .rposition(|line| !line.is_empty())
        .map_or(0, |last| last + 1);
    lines[end.saturating_sub(SCREEN_LINES)..end].join("\n")
}

pub fn judge(pane: PaneId, condition: &str, screen: String, results: mpsc::Sender<Judgment>) {
    let prompt = format!(
        "You watch a terminal pane for a person who parked it and asked to be told when a condition holds. \
         Judge only from the screen below. If the screen does not show the condition is true, it is not met.\n\
         Condition: {condition}\n\
         Screen (the last lines of the pane):\n{screen}\n\
         Reply with met, and a reason of at most 12 words saying what on the screen shows it."
    );
    thread::spawn(move || {
        let (outcome, tokens) = match run_claude(&prompt, &std::env::temp_dir(), &SCREEN_JUDGE) {
            Ok(reply) => reply.verdict(),
            Err(error) => (Err(error), 0),
        };
        let _ = results.send(Judgment {
            pane,
            outcome,
            tokens,
            review: false,
        });
    });
}

#[derive(Deserialize)]
struct ClaudeReply {
    #[serde(default)]
    is_error: bool,
    #[serde(default)]
    result: String,
    structured_output: Option<VerdictReply>,
    #[serde(default)]
    usage: Usage,
}

#[derive(Deserialize)]
struct VerdictReply {
    met: bool,
    reason: String,
}

#[derive(Deserialize, Default)]
struct Usage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
}

impl ClaudeReply {
    fn verdict(self) -> (Result<(bool, String), String>, u64) {
        let usage = &self.usage;
        let tokens = usage.input_tokens
            + usage.output_tokens
            + usage.cache_creation_input_tokens
            + usage.cache_read_input_tokens;
        let outcome = match self.structured_output {
            Some(verdict) if !self.is_error => Ok((verdict.met, verdict.reason.trim().to_string())),
            _ => Err(first_line(&self.result, "Claude gave no verdict")),
        };
        (outcome, tokens)
    }
}

struct Judge {
    model: &'static str,
    can_inspect: bool,
    timeout: Duration,
}

const SCREEN_JUDGE: Judge = Judge {
    model: "haiku",
    can_inspect: false,
    timeout: JUDGE_TIMEOUT,
};

const REVIEWER: Judge = Judge {
    model: "sonnet",
    can_inspect: true,
    timeout: REVIEW_TIMEOUT,
};

const READ_ONLY_COMMANDS: &[&str] = &[
    "git status",
    "git diff",
    "git log",
    "git show",
    "git blame",
    "git rev-parse",
    "git cat-file",
    "git ls-files",
    "git ls-remote",
    "git merge-base",
    "gh pr view",
    "gh pr diff",
    "gh pr checks",
    "gh pr list",
    "gh run view",
    "gh run list",
    "gh issue view",
];

fn tool_args(judge: &Judge) -> Vec<String> {
    if !judge.can_inspect {
        return vec!["--tools".into(), String::new()];
    }
    let commands = READ_ONLY_COMMANDS
        .iter()
        .map(|command| format!("Bash({command}:*)"));
    let allowed = ["Read", "Grep", "Glob"]
        .map(String::from)
        .into_iter()
        .chain(commands)
        .collect::<Vec<_>>()
        .join(" ");
    vec![
        "--tools".into(),
        "Read,Grep,Glob,Bash".into(),
        "--allowedTools".into(),
        allowed,
    ]
}

/// Without ANTHROPIC_API_KEY, claude falls back to the user's own login.
fn run_claude(prompt: &str, cwd: &Path, judge: &Judge) -> Result<ClaudeReply, String> {
    let mut child = Command::new("claude")
        .args(["-p", "--model", judge.model, "--no-session-persistence"])
        .args(tool_args(judge))
        .args([
            "--strict-mcp-config",
            "--setting-sources",
            "",
            "--output-format",
            "json",
            "--json-schema",
            VERDICT_SCHEMA,
            prompt,
        ])
        .current_dir(cwd)
        .env_remove("ANTHROPIC_API_KEY")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not start claude: {error}"))?;

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < judge.timeout => {
                thread::sleep(Duration::from_millis(200));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("claude took too long to judge".into());
            }
            Err(error) => return Err(format!("claude failed: {error}")),
        }
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_string(&mut stdout);
    }
    if let Some(mut err) = child.stderr.take() {
        let _ = err.read_to_string(&mut stderr);
    }
    serde_json::from_str::<ClaudeReply>(&stdout).map_err(|_| {
        let detail = if stderr.trim().is_empty() {
            &stdout
        } else {
            &stderr
        };
        first_line(detail, &format!("claude exited with {status}"))
    })
}

fn first_line(text: &str, fallback: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

pub fn age_label(at: SystemTime) -> String {
    let seconds = SystemTime::now()
        .duration_since(at)
        .unwrap_or_default()
        .as_secs();
    match seconds {
        0..60 => "just now".into(),
        60..3600 => format!("{}m ago", seconds / 60),
        3600..86400 => format!("{}h ago", seconds / 3600),
        _ => format!("{}d ago", seconds / 86400),
    }
}
