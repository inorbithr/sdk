//! The events of an agent's JSON stream and the one renderer that draws them (PRD 0088.1).
//!
//! Claude (`claude -p --output-format stream-json`) and Codex (`codex exec --json`) each print
//! one JSON object per line. Each line is read into its own typed serde enum below and
//! mapped to the single [`Event`] enum; [`Renderer`] draws [`Event`]s and knows no agent.
//! Anything a typed enum does not name is ignored, so a newer agent that adds events keeps
//! working. Nothing here logs, stores or traces the question or the answer: text goes from
//! the agent's pipe to the terminal and nowhere else.
//!
//! Sources of the shapes (read 2026-11-05):
//! - Claude: `claude --help` on the machine, and the lines a real `claude -p
//!   --output-format stream-json --verbose --include-partial-messages` run printed.
//! - Codex: <https://developers.openai.com/codex/noninteractive> (flags, event names) and
//!   `codex-rs/exec/src/exec_events.rs` in github.com/openai/codex (the item fields).

use std::io::Write;

use serde::Deserialize;
use serde_json::Value;

use super::Agent;

/// What the agent did, in the one vocabulary the renderer draws.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Event {
    /// A thinking block began.
    ThinkingStart,
    /// A piece of the agent's thinking, as it arrives.
    Thinking(String),
    /// A piece of the answer, as it arrives.
    Text(String),
    /// The agent used a tool: its name and the one thing it acted on. `id` ties it to its
    /// [`Event::ToolResult`] when the agent sends one (Claude does, Codex does not).
    Tool {
        id: Option<String>,
        name: String,
        detail: String,
    },
    /// A tool's result. `denied` is the reason when the agent refused the call (the tool is
    /// disabled, or the call is not on the allow list and nobody can be asked): the call did
    /// not run, so it is never drawn as one.
    ToolResult { id: String, denied: Option<String> },
    /// The run ended.
    Done(Done),
}

/// The end of a run.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Done {
    pub(crate) duration_ms: Option<u64>,
    pub(crate) turns: Option<u64>,
    pub(crate) cost_usd: Option<f64>,
    pub(crate) thinking_tokens: Option<u64>,
    /// The whole answer, for an agent that sent no pieces.
    pub(crate) answer: Option<String>,
    /// Set when the agent says the run failed: its short reason.
    pub(crate) error: Option<String>,
    /// Set when the run stopped at its turn cap (`error_max_turns`): the work is on disk, unfinished.
    pub(crate) capped: bool,
}

/// Reads one line of the agent's stream. A line that is not JSON, or an event this build does
/// not know, is no event.
#[must_use]
pub(crate) fn parse(agent: Agent, line: &str) -> Vec<Event> {
    match agent {
        Agent::Claude => serde_json::from_str::<ClaudeLine>(line)
            .map(ClaudeLine::events)
            .unwrap_or_default(),
        Agent::Codex => serde_json::from_str::<CodexLine>(line)
            .map(CodexLine::events)
            .unwrap_or_default(),
    }
}

// ---- Claude ----------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ClaudeLine {
    #[serde(rename = "stream_event")]
    Stream {
        event: ClaudeStream,
        #[serde(default)]
        parent_tool_use_id: Option<String>,
    },
    #[serde(rename = "assistant")]
    Assistant {
        message: ClaudeMessage,
        #[serde(default)]
        parent_tool_use_id: Option<String>,
    },
    /// The tool results the agent sends back to itself.
    #[serde(rename = "user")]
    User {
        message: ClaudeMessage,
        #[serde(default)]
        parent_tool_use_id: Option<String>,
    },
    #[serde(rename = "result")]
    Result(ClaudeResult),
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ClaudeStream {
    #[serde(rename = "content_block_start")]
    BlockStart { content_block: ClaudeBlock },
    #[serde(rename = "content_block_delta")]
    BlockDelta { delta: ClaudeDelta },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ClaudeBlock {
    #[serde(rename = "thinking")]
    Thinking,
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ClaudeDelta {
    #[serde(rename = "thinking_delta")]
    Thinking {
        #[serde(default)]
        thinking: String,
    },
    #[serde(rename = "text_delta")]
    Text {
        #[serde(default)]
        text: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct ClaudeMessage {
    #[serde(default)]
    content: Vec<ClaudeContent>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ClaudeContent {
    #[serde(rename = "tool_use")]
    ToolUse {
        #[serde(default)]
        id: Option<String>,
        name: String,
        #[serde(default)]
        input: Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        #[serde(default)]
        tool_use_id: String,
        #[serde(default)]
        content: Value,
        #[serde(default)]
        is_error: bool,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct ClaudeResult {
    #[serde(default)]
    is_error: bool,
    /// `success`, or `error_max_turns`, `error_during_execution`, ...
    #[serde(default)]
    subtype: Option<String>,
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    duration_ms: Option<u64>,
    #[serde(default)]
    num_turns: Option<u64>,
    #[serde(default)]
    total_cost_usd: Option<f64>,
    #[serde(default)]
    usage: Option<ClaudeUsage>,
}

#[derive(Debug, Deserialize)]
struct ClaudeUsage {
    #[serde(default)]
    output_tokens_details: Option<ClaudeOutputDetails>,
}

#[derive(Debug, Deserialize)]
struct ClaudeOutputDetails {
    #[serde(default)]
    thinking_tokens: Option<u64>,
}

impl ClaudeLine {
    fn events(self) -> Vec<Event> {
        match self {
            // A sub-agent's own messages are not the answer to this question.
            Self::Stream {
                parent_tool_use_id: Some(_),
                ..
            }
            | Self::Assistant {
                parent_tool_use_id: Some(_),
                ..
            }
            | Self::User {
                parent_tool_use_id: Some(_),
                ..
            }
            | Self::Other => Vec::new(),
            Self::Stream { event, .. } => match event {
                ClaudeStream::BlockStart {
                    content_block: ClaudeBlock::Thinking,
                } => vec![Event::ThinkingStart],
                ClaudeStream::BlockDelta {
                    delta: ClaudeDelta::Thinking { thinking },
                } if !thinking.is_empty() => vec![Event::Thinking(thinking)],
                ClaudeStream::BlockDelta {
                    delta: ClaudeDelta::Text { text },
                } if !text.is_empty() => vec![Event::Text(text)],
                _ => Vec::new(),
            },
            // The finished message carries each tool's whole input; the stream only its pieces.
            Self::Assistant { message, .. } => message
                .content
                .into_iter()
                .filter_map(|c| match c {
                    ClaudeContent::ToolUse { id, name, input } => Some(Event::Tool {
                        id,
                        detail: tool_detail(&input),
                        name,
                    }),
                    _ => None,
                })
                .collect(),
            // A result that says the call was refused: it never ran.
            Self::User { message, .. } => message
                .content
                .into_iter()
                .filter_map(|c| match c {
                    ClaudeContent::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    } if !tool_use_id.is_empty() => Some(Event::ToolResult {
                        denied: is_error.then(|| denial(&content)).flatten(),
                        id: tool_use_id,
                    }),
                    _ => None,
                })
                .collect(),
            Self::Result(r) => {
                let stopped = r.subtype.as_deref().filter(|t| t.starts_with("error"));
                let failed = r.is_error || stopped.is_some();
                let reason = match stopped {
                    Some("error_max_turns") => {
                        Some("it used all its turns before it finished".to_owned())
                    }
                    _ => r.result.clone(),
                };
                vec![Event::Done(Done {
                    duration_ms: r.duration_ms,
                    turns: r.num_turns,
                    cost_usd: r.total_cost_usd,
                    thinking_tokens: r
                        .usage
                        .and_then(|u| u.output_tokens_details)
                        .and_then(|d| d.thinking_tokens),
                    error: failed.then(|| {
                        reason.unwrap_or_else(|| "the agent reported an error".to_owned())
                    }),
                    answer: if failed { None } else { r.result },
                    capped: stopped == Some("error_max_turns"),
                })]
            }
        }
    }
}

// ---- Codex -----------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum CodexLine {
    #[serde(rename = "item.started")]
    ItemStarted { item: CodexItem },
    #[serde(rename = "item.completed")]
    ItemCompleted { item: CodexItem },
    #[serde(rename = "turn.completed")]
    TurnCompleted {
        #[serde(default)]
        usage: Option<CodexUsage>,
    },
    #[serde(rename = "turn.failed")]
    TurnFailed { error: CodexError },
    #[serde(rename = "error")]
    Error { message: String },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct CodexError {
    message: String,
}

#[derive(Debug, Deserialize)]
struct CodexUsage {
    #[serde(default)]
    reasoning_output_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum CodexItem {
    AgentMessage {
        #[serde(default)]
        text: String,
    },
    Reasoning {
        #[serde(default)]
        text: String,
    },
    CommandExecution {
        #[serde(default)]
        command: String,
    },
    FileChange {
        #[serde(default)]
        changes: Vec<CodexChange>,
    },
    McpToolCall {
        #[serde(default)]
        server: String,
        #[serde(default)]
        tool: String,
    },
    WebSearch {
        #[serde(default)]
        query: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct CodexChange {
    #[serde(default)]
    path: String,
}

impl CodexLine {
    fn events(self) -> Vec<Event> {
        match self {
            // A command, tool or search is drawn when it starts, a file change when it is done
            // (Codex sends it only then), a message or reasoning when it is whole.
            Self::ItemStarted { item } => match item {
                CodexItem::CommandExecution { command } => vec![Event::Tool {
                    id: None,
                    name: "Run".to_owned(),
                    detail: short(&command),
                }],
                CodexItem::McpToolCall { server, tool } => vec![Event::Tool {
                    id: None,
                    name: format!("{server}.{tool}"),
                    detail: String::new(),
                }],
                CodexItem::WebSearch { query } => vec![Event::Tool {
                    id: None,
                    name: "WebSearch".to_owned(),
                    detail: short(&query),
                }],
                _ => Vec::new(),
            },
            Self::ItemCompleted { item } => match item {
                CodexItem::AgentMessage { text } if !text.is_empty() => {
                    vec![Event::Text(format!("{text}\n"))]
                }
                CodexItem::Reasoning { text } if !text.is_empty() => {
                    vec![Event::ThinkingStart, Event::Thinking(format!("{text}\n"))]
                }
                CodexItem::FileChange { changes } => vec![Event::Tool {
                    id: None,
                    name: "Edit".to_owned(),
                    detail: short(
                        &changes
                            .iter()
                            .map(|c| c.path.as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                }],
                _ => Vec::new(),
            },
            Self::TurnCompleted { usage } => vec![Event::Done(Done {
                turns: Some(1),
                thinking_tokens: usage.and_then(|u| u.reasoning_output_tokens),
                ..Done::default()
            })],
            Self::TurnFailed { error } => vec![Event::Done(Done {
                error: Some(error.message),
                ..Done::default()
            })],
            Self::Error { message } => vec![Event::Done(Done {
                error: Some(message),
                ..Done::default()
            })],
            Self::Other => Vec::new(),
        }
    }
}

// ---- Shared ----------------------------------------------------------------------------

/// The one thing a tool acted on, from its input: a file, a pattern, a command, an address.
fn tool_detail(input: &Value) -> String {
    [
        "file_path",
        "path",
        "pattern",
        "command",
        "url",
        "query",
        "description",
        "prompt",
    ]
    .iter()
    .find_map(|k| input.get(*k).and_then(Value::as_str))
    .map(|s| s.lines().next().unwrap_or_default().trim().to_owned())
    .unwrap_or_default()
}

/// The reason a tool result gives for a refused call, when it is a refusal: Claude Code says
/// `No such tool available: Bash. Bash is disabled for this session` for a tool taken away
/// with `--disallowedTools`, and a permission message (`... requested permissions to use X,
/// but you haven't granted it yet`, `Permission to use X has been denied`) for a call that is
/// not on the allow list when nobody can be asked. Any other failed result (a missing file, a
/// failing command) is a tool that ran and failed, and is `None`.
fn denial(content: &Value) -> Option<String> {
    let text = match content {
        Value::String(s) => s.clone(),
        Value::Array(a) => a
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    };
    let lower = text.to_ascii_lowercase();
    let refused = [
        "no such tool available",
        "is disabled",
        "permission to use",
        "requested permissions",
        "haven't granted",
        "has been denied",
        "denied by your permission",
        "was denied",
        "requires approval",
        "not allowed",
    ]
    .iter()
    .any(|m| lower.contains(m));
    refused.then(|| {
        let first = text
            .replace("<tool_use_error>", "")
            .replace("</tool_use_error>", "");
        let first = first.trim().trim_start_matches("Error:").trim();
        short(first.lines().next().unwrap_or_default())
    })
}

/// At most 100 characters, cut with an ellipsis; applied once the path is shortened.
fn short(s: &str) -> String {
    if s.chars().count() > 100 {
        let cut: String = s.chars().take(99).collect();
        format!("{cut}\u{2026}")
    } else {
        s.to_owned()
    }
}

/// How the thinking is drawn on a terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Thinking {
    /// Streamed as it arrives, dim and italic, in a quiet block with a gutter.
    Show,
    /// One collapsed line; the text is not drawn.
    Collapse,
}

/// How a run is drawn: the look, then what the footer names.
#[derive(Debug, Clone, Default)]
pub(crate) struct Look {
    /// The drawn look (a terminal on stderr, colour allowed). Plain: only the answer.
    pub(crate) fancy: bool,
    pub(crate) thinking: Option<Thinking>,
    /// The folder the agent runs in, to show paths inside it without it.
    pub(crate) cwd: Option<String>,
    /// The model asked for, shown in the footer.
    pub(crate) model: Option<String>,
    /// A run nobody watches live (`iohr project init`): without the drawn look, still print one
    /// plain row per tool and the footer, on stderr, so a log shows what the agent did.
    pub(crate) rows: bool,
    /// What the footer says the AI made: `AI answer` when unset.
    pub(crate) made: Option<&'static str>,
    /// The live view of a run a person waits on (`iohr project init`), only with `fancy`: a
    /// spinner on the tool that runs now, a check mark row for each tool that finished, and the
    /// agent's closing words held back and drawn as rendered Markdown once, with the footer.
    /// Narration between tool calls is not drawn; the summary is what the agent says last.
    pub(crate) summary: bool,
}

/// A string in the dim style of the terminal on stderr.
fn dim(s: &str) -> String {
    console::style(s).for_stderr().dim().to_string()
}

/// What a caller wants to know of how a run ended, besides how it was drawn: did it stop at its
/// turn cap, and what were the agent's last words (the text since its last tool call, else the
/// text before it). Held in memory for one run, for the handoff note; never logged.
#[derive(Debug, Default)]
pub(crate) struct Tail {
    pub(crate) capped: bool,
    run: String,
    last: String,
}

/// The most of the agent's words kept; the note cuts it again.
const TAIL_MAX: usize = 6000;

impl Tail {
    pub(crate) fn see(&mut self, e: &Event) {
        match e {
            Event::Text(t) => {
                if self.run.len() < TAIL_MAX {
                    self.run.push_str(t);
                }
            }
            Event::Tool { .. } => {
                if !self.run.trim().is_empty() {
                    self.last = std::mem::take(&mut self.run);
                }
                self.run.clear();
            }
            Event::Done(d) => {
                self.capped |= d.capped;
                if let Some(a) = d.answer.as_deref().filter(|a| !a.trim().is_empty()) {
                    a.clone_into(&mut self.run);
                }
            }
            _ => {}
        }
    }

    /// The agent's last words, when it said any.
    pub(crate) fn said(&self) -> String {
        if self.run.trim().is_empty() {
            self.last.trim().to_owned()
        } else {
            self.run.trim().to_owned()
        }
    }
}

/// Draws [`Event`]s. The answer goes to `out` (stdout) and nothing else does, so a pipe gets
/// only the answer. When `fancy`, the thinking, the tool rows and the footer go to `err`
/// (stderr) in the look of [`crate::ui`] (`console` styles, the cliclack gutter and spinner);
/// plain, they are not drawn at all.
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent drawing states of one run, not a set of options"
)]
pub(crate) struct Renderer<O: Write, E: Write> {
    out: O,
    err: E,
    look: Look,
    /// Shown until the first event arrives.
    spinner: Option<crate::ui::Step>,
    started: std::time::Instant,
    /// The answer so far ends mid-line.
    out_mid: bool,
    /// The thinking block is open and its last line is unfinished.
    err_mid: bool,
    /// The thinking block is open and the next text starts a line.
    err_line_start: bool,
    /// A tool ran since the last piece of the answer.
    broke: bool,
    printed_text: bool,
    failed: Option<String>,
    /// Tool calls drawn only when their result says whether they ran: id, name, detail.
    pending: Vec<(String, String, String)>,
    /// The live view: the spinner that shows what runs now, and the closing words held back.
    live: Option<Live>,
}

/// The state of the live view of a run ([`Look::summary`]).
#[derive(Default)]
struct Live {
    bar: Option<crate::ui::Step>,
    text: String,
}

impl<O: Write, E: Write> Renderer<O, E> {
    pub(crate) fn new(out: O, err: E, look: Look, spinner: Option<crate::ui::Step>) -> Self {
        let look_live = look.fancy && look.summary;
        Self {
            out,
            err,
            look,
            spinner,
            started: std::time::Instant::now(),
            out_mid: false,
            err_mid: false,
            err_line_start: false,
            broke: false,
            printed_text: false,
            failed: None,
            pending: Vec::new(),
            live: (look_live).then(Live::default),
        }
    }

    /// The agent's own reason, when it said the run failed.
    pub(crate) fn failure(&self) -> Option<&str> {
        self.failed.as_deref()
    }

    fn stop_spinner(&mut self) {
        if let Some(s) = self.spinner.take() {
            s.clear();
        }
    }

    /// Draws one event. A closed pipe is not an error.
    pub(crate) fn draw(&mut self, event: Event) {
        if self.live.is_some() {
            self.draw_live(event);
            return;
        }
        self.stop_spinner();
        // A tool whose result never came (the run ended, or the agent moved on) is drawn as
        // the call it was, before the next thing is.
        if !matches!(event, Event::Tool { .. } | Event::ToolResult { .. }) {
            self.flush_tools();
        }
        match event {
            Event::ThinkingStart => {
                if self.look.fancy {
                    self.end_thinking_line();
                    self.end_out_line();
                    let collapsed = self.look.thinking == Some(Thinking::Collapse);
                    let arrow = if collapsed { "\u{25b8}" } else { "\u{25be}" };
                    let _ = writeln!(
                        self.err,
                        "{}",
                        console::style(format!("{arrow} thinking"))
                            .for_stderr()
                            .dim()
                            .italic()
                    );
                    self.err_line_start = true;
                }
            }
            Event::Thinking(t) => {
                if self.look.fancy && self.look.thinking != Some(Thinking::Collapse) {
                    self.thinking_text(&t);
                }
            }
            Event::Text(t) => {
                self.end_thinking_line();
                if self.broke && self.out_mid {
                    let _ = writeln!(self.out);
                    self.out_mid = false;
                }
                self.broke = false;
                let _ = self.out.write_all(t.as_bytes());
                let _ = self.out.flush();
                self.printed_text = true;
                self.out_mid = !t.ends_with('\n');
            }
            Event::Tool {
                id: Some(id),
                name,
                detail,
            } => self.pending.push((id, name, detail)),
            Event::Tool { name, detail, .. } => self.tool_row(&name, &detail, None),
            Event::ToolResult { id, denied } => {
                if let Some(at) = self.pending.iter().position(|p| p.0 == id) {
                    let (_, name, detail) = self.pending.remove(at);
                    self.tool_row(&name, &detail, denied.as_deref());
                }
            }
            Event::Done(d) => self.done(d),
        }
    }

    fn flush_tools(&mut self) {
        for (_, name, detail) in std::mem::take(&mut self.pending) {
            self.tool_row(&name, &detail, None);
        }
    }

    /// One tool row. A call the agent refused is drawn dim, marked `\u{2298}` and with its
    /// reason, and never like a call that ran.
    fn tool_row(&mut self, name: &str, detail: &str, denied: Option<&str>) {
        self.broke = true;
        let detail = short(&self.relative(detail));
        if !self.look.fancy && self.look.rows {
            self.end_out_line();
            let _ = match denied {
                Some(why) => writeln!(self.err, "- {name} {detail} (denied, did not run: {why})"),
                None => writeln!(self.err, "- {name} {detail}"),
            };
        } else if self.look.fancy {
            self.end_thinking_line();
            self.end_out_line();
            let _ = match denied {
                Some(why) => writeln!(
                    self.err,
                    "{}",
                    // Cut to the terminal's width so the row never wraps.
                    dim(&console::truncate_str(
                        &format!("\u{2298} {name} {detail} \u{2014} denied, did not run: {why}"),
                        crate::ui::terminal_width().saturating_sub(1).max(40),
                        "\u{2026}",
                    ))
                ),
                None => writeln!(
                    self.err,
                    "{} {} {}",
                    console::style("\u{23fa}").for_stderr().cyan(),
                    console::style(name).for_stderr().bold(),
                    dim(&detail)
                ),
            };
        }
    }

    /// Streams thinking in the gutter, one `\u{2502} ` per line, dim and italic.
    fn thinking_text(&mut self, t: &str) {
        for piece in t.split_inclusive('\n') {
            if self.err_line_start {
                let _ = write!(self.err, "{}", dim("\u{2502} "));
                self.err_line_start = false;
            }
            let _ = write!(
                self.err,
                "{}",
                console::style(piece.trim_end_matches('\n'))
                    .for_stderr()
                    .dim()
                    .italic()
            );
            if piece.ends_with('\n') {
                let _ = writeln!(self.err);
                self.err_line_start = true;
                self.err_mid = false;
            } else {
                self.err_mid = true;
            }
        }
        let _ = self.err.flush();
    }

    fn done(&mut self, d: Done) {
        if let (false, Some(answer)) = (self.printed_text, d.answer.as_deref()) {
            // The agent sent no pieces: the whole answer is the one thing to print.
            let _ = writeln!(self.out, "{answer}");
            self.printed_text = true;
            self.out_mid = false;
        }
        self.end_thinking_line();
        self.end_out_line();
        if let Some(e) = d.error {
            self.failed = Some(e);
            return;
        }
        self.footer(&d);
    }

    /// The footer: model, time, turns, cost and what the AI made, dim.
    fn footer(&mut self, d: &Done) {
        if self.look.fancy || self.look.rows {
            let rail = if self.live.is_some() {
                "\u{2502}  "
            } else {
                ""
            };
            let ms = d.duration_ms.unwrap_or_else(|| {
                u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
            });
            let mut parts = Vec::new();
            if let Some(m) = &self.look.model {
                parts.push(m.clone());
            }
            parts.push(format!("{}.{}s", ms / 1000, ms % 1000 / 100));
            if let Some(t) = d.turns {
                parts.push(format!("{t} turn{}", if t == 1 { "" } else { "s" }));
            }
            if let Some(c) = d.cost_usd {
                parts.push(format!("${c:.4}"));
            }
            if let Some(t) = d.thinking_tokens.filter(|t| *t > 0) {
                parts.push(format!("{t} thinking tokens"));
            }
            parts.push(self.look.made.unwrap_or("AI answer").to_owned());
            let _ = writeln!(
                self.err,
                "{}",
                dim(&format!("{rail}{}", parts.join(" \u{b7} ")))
            );
        }
    }

    /// Ends the run: the answer finishes on a line of its own.
    pub(crate) fn finish(&mut self) {
        self.stop_spinner();
        self.live_end();
        self.flush_tools();
        self.end_thinking_line();
        self.end_out_line();
    }

    fn end_out_line(&mut self) {
        if self.out_mid {
            let _ = writeln!(self.out);
            let _ = self.out.flush();
            self.out_mid = false;
        }
    }

    fn end_thinking_line(&mut self) {
        if self.err_mid {
            let _ = writeln!(self.err);
            self.err_mid = false;
        }
        self.err_line_start = false;
    }

    /// A path inside the folder `iohr ask` runs in is shown without it.
    fn relative(&self, detail: &str) -> String {
        self.look
            .cwd
            .as_deref()
            .and_then(|c| detail.strip_prefix(c))
            .and_then(|r| r.strip_prefix('/'))
            .unwrap_or(detail)
            .to_owned()
    }
}

/// A finished tool call as one live row: a check mark, the tool and what it touched, cut to
/// `width`. A call the agent was refused is dim, marked `\u{2298}`, with the reason.
fn live_row(name: &str, detail: &str, denied: Option<&str>, width: usize) -> String {
    if let Some(why) = denied {
        return dim(&console::truncate_str(
            &format!("\u{2298} {name} {detail} \u{2014} denied, did not run: {why}"),
            width,
            "\u{2026}",
        ));
    }
    let full = format!("{name} {detail}");
    let said = console::truncate_str(&full, width.saturating_sub(3), "\u{2026}");
    let (head, rest) = said
        .split_once(' ')
        .map_or((said.as_ref(), ""), |(h, r)| (h, r));
    format!(
        "{}  {} {}",
        console::style("\u{2713}").for_stderr().green().bold(),
        console::style(head).for_stderr().bold(),
        dim(rest)
    )
}

/// The agent's closing words as the lines of the summary block: Markdown drawn for a terminal,
/// wrapped to `width` less the rail, each line behind the rail.
fn summary_lines(text: &str, width: usize) -> Vec<String> {
    let rail = dim("\u{2502}  ");
    crate::card::body(
        text.trim(),
        width.saturating_sub(3).max(30),
        crate::card::Look::detect(true),
    )
    .into_iter()
    .map(|l| {
        if l.is_empty() {
            dim("\u{2502}")
        } else {
            format!("{rail}{l}")
        }
    })
    .collect()
}

impl<O: Write, E: Write> Renderer<O, E> {
    /// Says what runs now on the live spinner, starting it when it is not showing.
    fn live_say(&mut self, what: &str) {
        let what = console::truncate_str(
            what,
            crate::ui::terminal_width().saturating_sub(8),
            "\u{2026}",
        );
        if let Some(live) = self.live.as_mut() {
            match &live.bar {
                Some(b) => b.say(&what),
                None => live.bar = Some(crate::ui::Ui::detect(false).step(&what)),
            }
        }
    }

    /// The spinner is gone for a moment, so a row can be drawn above it.
    fn live_pause(&mut self) {
        if let Some(b) = self.live.as_mut().and_then(|l| l.bar.take()) {
            b.clear();
        }
    }

    fn live_tool_row(&mut self, name: &str, detail: &str, denied: Option<&str>) {
        self.live_pause();
        let detail = short(&self.relative(detail));
        let row = live_row(
            name,
            &detail,
            denied,
            crate::ui::terminal_width().saturating_sub(1),
        );
        let _ = writeln!(self.err, "{row}");
        let _ = self.err.flush();
        self.live_say("Thinking");
    }

    fn draw_live(&mut self, event: Event) {
        // The spinner the caller started stands in until the first event; the live one takes over.
        self.stop_spinner();
        match event {
            Event::ThinkingStart | Event::Thinking(_) => self.live_say("Thinking"),
            Event::Text(t) => {
                if let Some(live) = self.live.as_mut() {
                    live.text.push_str(&t);
                }
                self.live_say("Writing the summary");
            }
            Event::Tool {
                id: Some(id),
                name,
                detail,
            } => {
                // What was said before a tool call is narration, not the summary.
                if let Some(live) = self.live.as_mut() {
                    live.text.clear();
                }
                let shown = short(&self.relative(&detail));
                self.live_say(&format!("{name} {shown}"));
                self.pending.push((id, name, detail));
            }
            Event::Tool { name, detail, .. } => {
                if let Some(live) = self.live.as_mut() {
                    live.text.clear();
                }
                self.broke = true;
                self.live_tool_row(&name, &detail, None);
            }
            Event::ToolResult { id, denied } => {
                if let Some(at) = self.pending.iter().position(|p| p.0 == id) {
                    let (_, name, detail) = self.pending.remove(at);
                    self.live_tool_row(&name, &detail, denied.as_deref());
                }
            }
            Event::Done(d) => {
                for (_, name, detail) in std::mem::take(&mut self.pending) {
                    self.live_tool_row(&name, &detail, None);
                }
                self.live_pause();
                if let Some(e) = d.error {
                    self.failed = Some(e);
                    return;
                }
                if let (Some(live), Some(a)) = (self.live.as_mut(), d.answer.as_deref())
                    && live.text.trim().is_empty()
                {
                    a.clone_into(&mut live.text);
                }
                self.live_summary();
                self.footer(&d);
            }
        }
    }

    /// Draws the held closing words, once.
    fn live_summary(&mut self) {
        let text = self
            .live
            .as_mut()
            .map(|l| std::mem::take(&mut l.text))
            .unwrap_or_default();
        if text.trim().is_empty() {
            return;
        }
        let _ = writeln!(self.err, "{}", dim("\u{2502}"));
        for l in summary_lines(&text, crate::ui::terminal_width()) {
            let _ = writeln!(self.err, "{l}");
        }
        let _ = writeln!(self.err, "{}", dim("\u{2502}"));
        self.printed_text = true;
    }

    /// The run ended without a result event (a closed stream): what was held back is drawn
    /// now, and the spinner goes.
    fn live_end(&mut self) {
        if self.live.is_none() {
            return;
        }
        self.live_pause();
        for (_, name, detail) in std::mem::take(&mut self.pending) {
            self.live_tool_row(&name, &detail, None);
        }
        self.live_pause();
        self.live_summary();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude(line: &str) -> Vec<Event> {
        parse(Agent::Claude, line)
    }

    #[test]
    fn claude_text_thinking_and_tools_become_events() {
        assert_eq!(
            claude(
                r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}}"#
            ),
            [Event::Text("Hi".into())]
        );
        assert_eq!(
            claude(
                r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}}"#
            ),
            [Event::ThinkingStart]
        );
        // An empty thinking piece (the model sent none) is no event.
        assert_eq!(
            claude(
                r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"thinking_delta","thinking":""}}}"#
            ),
            []
        );
        assert_eq!(
            claude(
                r#"{"type":"assistant","message":{"content":[{"type":"text","text":"x"},{"type":"tool_use","name":"Read","input":{"file_path":"README.md"}}]}}"#
            ),
            [Event::Tool {
                id: None,
                name: "Read".into(),
                detail: "README.md".into()
            }]
        );
    }

    #[test]
    fn unknown_lines_and_sub_agents_are_ignored() {
        assert_eq!(claude(r#"{"type":"rate_limit_event","x":1}"#), []);
        assert_eq!(
            claude(r#"{"type":"stream_event","event":{"type":"brand_new"}}"#),
            []
        );
        assert_eq!(claude("not json"), []);
        assert_eq!(
            claude(
                r#"{"type":"stream_event","parent_tool_use_id":"t1","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"sub"}}}"#
            ),
            []
        );
    }

    #[test]
    fn running_out_of_turns_is_a_failure_with_a_plain_reason() {
        let ev = claude(
            r#"{"type":"result","subtype":"error_max_turns","is_error":false,"num_turns":40}"#,
        );
        let [Event::Done(d)] = ev.as_slice() else {
            panic!("one done event: {ev:?}");
        };
        assert_eq!(
            d.error.as_deref(),
            Some("it used all its turns before it finished")
        );
        assert_eq!(d.turns, Some(40));
        assert!(d.capped, "the turn cap is told apart from other failures");
        let ev = claude(r#"{"type":"result","is_error":true,"result":"Not logged in"}"#);
        assert!(matches!(&ev[0], Event::Done(d) if !d.capped));
    }

    #[test]
    fn the_tail_keeps_the_last_words_and_the_cap() {
        let mut t = Tail::default();
        for e in [
            Event::Text("Wrote the PRD. Next: the ADRs.".into()),
            Event::Tool {
                id: None,
                name: "Write".into(),
                detail: "atlas/adr/0002.md".into(),
            },
            Event::Done(Done {
                capped: true,
                ..Done::default()
            }),
        ] {
            t.see(&e);
        }
        assert!(t.capped);
        assert_eq!(t.said(), "Wrote the PRD. Next: the ADRs.");
        t.see(&Event::Text("Then this.".into()));
        assert_eq!(t.said(), "Then this.");
    }

    #[test]
    fn a_result_carries_the_footer_numbers_and_a_failure() {
        let ev = claude(
            r#"{"type":"result","is_error":false,"result":"ok","duration_ms":1500,"num_turns":2,"total_cost_usd":0.01,"usage":{"output_tokens_details":{"thinking_tokens":7}}}"#,
        );
        assert_eq!(
            ev,
            [Event::Done(Done {
                duration_ms: Some(1500),
                turns: Some(2),
                cost_usd: Some(0.01),
                thinking_tokens: Some(7),
                answer: Some("ok".into()),
                error: None,
                capped: false,
            })]
        );
        let ev = claude(r#"{"type":"result","is_error":true,"result":"Not logged in"}"#);
        assert!(matches!(&ev[0], Event::Done(d) if d.error.as_deref() == Some("Not logged in")));
    }

    #[test]
    fn codex_items_map_to_the_same_events() {
        let c = |l: &str| parse(Agent::Codex, l);
        assert_eq!(
            c(
                r#"{"type":"item.started","item":{"id":"i1","type":"command_execution","command":"bash -lc ls","status":"in_progress"}}"#
            ),
            [Event::Tool {
                id: None,
                name: "Run".into(),
                detail: "bash -lc ls".into()
            }]
        );
        // The same command's completion draws nothing a second time.
        assert_eq!(
            c(
                r#"{"type":"item.completed","item":{"id":"i1","type":"command_execution","command":"bash -lc ls","exit_code":0,"status":"completed"}}"#
            ),
            []
        );
        assert_eq!(
            c(
                r#"{"type":"item.completed","item":{"id":"i2","type":"agent_message","text":"Hello"}}"#
            ),
            [Event::Text("Hello\n".into())]
        );
        assert_eq!(
            c(r#"{"type":"item.completed","item":{"id":"i3","type":"reasoning","text":"hm"}}"#),
            [Event::ThinkingStart, Event::Thinking("hm\n".into())]
        );
        assert_eq!(
            c(
                r#"{"type":"item.completed","item":{"id":"i4","type":"file_change","changes":[{"path":"a.rs","kind":"update"}],"status":"completed"}}"#
            ),
            [Event::Tool {
                id: None,
                name: "Edit".into(),
                detail: "a.rs".into()
            }]
        );
        assert_eq!(c(r#"{"type":"thread.started","thread_id":"t"}"#), []);
        assert_eq!(
            c(r#"{"type":"item.completed","item":{"type":"todo_list","items":[]}}"#),
            []
        );
        assert!(
            matches!(&c(r#"{"type":"turn.failed","error":{"message":"boom"}}"#)[0], Event::Done(d) if d.error.as_deref() == Some("boom"))
        );
    }

    fn draw(fancy: bool, events: Vec<Event>) -> (String, String) {
        let look = Look {
            fancy,
            thinking: Some(Thinking::Show),
            cwd: Some("/work".into()),
            model: Some("haiku".into()),
            rows: false,
            made: None,
            summary: false,
        };
        let mut r = Renderer::new(Vec::new(), Vec::new(), look, None);
        for e in events {
            r.draw(e);
        }
        r.finish();
        (
            String::from_utf8(r.out).unwrap_or_default(),
            String::from_utf8(r.err).unwrap_or_default(),
        )
    }

    /// Two lines from a real Claude Code 2.1.x run with `--disallowedTools Bash`: the call,
    /// then the result that says the tool does not exist in this session.
    const DENIED_CALL: &str = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_01","name":"Bash","input":{"command":"cd /work/demo && git status"}}]},"parent_tool_use_id":null}"#;
    const DENIED_RESULT: &str = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"<tool_use_error>Error: No such tool available: Bash. Bash is disabled for this session, in subagents as well as here.</tool_use_error>","is_error":true,"tool_use_id":"toolu_01"}]},"parent_tool_use_id":null}"#;

    #[test]
    fn a_refused_tool_result_is_a_denial_and_a_failing_command_is_not() {
        assert_eq!(
            claude(DENIED_RESULT),
            [Event::ToolResult {
                id: "toolu_01".into(),
                denied: Some(
                    "No such tool available: Bash. Bash is disabled for this session, in subagents as well as here."
                        .into()
                )
            }]
        );
        // The permission message for a call off the allow list.
        let off_list = r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"Claude requested permissions to use Bash, but you haven't granted it yet.","is_error":true,"tool_use_id":"t2"}]}}"#;
        assert!(matches!(
            &claude(off_list)[0],
            Event::ToolResult {
                denied: Some(_),
                ..
            }
        ));
        // A command that ran and failed, a file that is missing, and a result that is fine.
        for ok in [
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"Exit code 1\nerror[E0308]: mismatched types","is_error":true,"tool_use_id":"t3"}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"File does not exist.","is_error":true,"tool_use_id":"t4"}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"fine","tool_use_id":"t5"}]}}"#,
        ] {
            assert!(
                matches!(&claude(ok)[0], Event::ToolResult { denied: None, .. }),
                "{ok}"
            );
        }
    }

    #[test]
    fn a_denied_call_is_drawn_dim_with_its_reason_and_never_as_a_run_line() {
        let events: Vec<Event> = [DENIED_CALL, DENIED_RESULT]
            .into_iter()
            .flat_map(claude)
            .collect();
        let (_, fancy) = draw(true, events.clone());
        assert!(fancy.contains("\u{2298} Bash"), "{fancy}");
        assert!(
            fancy.contains("denied, did not run: No such tool"),
            "{fancy}"
        );
        assert!(!fancy.contains('\u{23fa}'), "never the run mark: {fancy}");
        let look = Look {
            fancy: false,
            thinking: None,
            cwd: Some("/work".into()),
            model: None,
            rows: true,
            made: None,
            summary: false,
        };
        let mut r = Renderer::new(Vec::new(), Vec::new(), look, None);
        for e in events {
            r.draw(e);
        }
        r.finish();
        let rows = String::from_utf8(r.err).unwrap_or_default();
        assert!(
            rows.contains("- Bash cd /work/demo && git status (denied, did not run: No such tool"),
            "{rows}"
        );
    }

    #[test]
    fn a_denied_row_never_runs_past_the_terminal_width() {
        let long = format!("which iohr cargo rustc rustup; {}", "echo x; ".repeat(30));
        let call = format!(
            r#"{{"type":"assistant","message":{{"content":[{{"type":"tool_use","id":"t9","name":"Bash","input":{{"command":"{long}"}}}}]}}}}"#
        );
        let result = r#"{"type":"user","message":{"content":[{"type":"tool_result","is_error":true,"content":"Permission to use Bash with command which iohr cargo rustc has been denied because it requires approval from somebody","tool_use_id":"t9"}]}}"#;
        let events: Vec<Event> = [call.as_str(), result]
            .into_iter()
            .flat_map(claude)
            .collect();
        let (_, fancy) = draw(true, events);
        let plain = console::strip_ansi_codes(&fancy);
        let row = plain
            .lines()
            .find(|l| l.starts_with('\u{2298}'))
            .unwrap_or_default();
        assert!(
            row.chars().count() <= crate::ui::terminal_width(),
            "{} chars: {row}",
            row.chars().count()
        );
        assert!(row.ends_with('\u{2026}'), "{row}");
    }

    #[test]
    fn a_call_that_ran_is_drawn_once_its_result_arrives_and_one_without_a_result_at_the_end() {
        let ran = r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"ok","tool_use_id":"toolu_01"}]}}"#;
        let events: Vec<Event> = [DENIED_CALL, ran].into_iter().flat_map(claude).collect();
        let (_, err) = draw(true, events);
        assert!(
            err.contains('\u{23fa}') && !err.contains('\u{2298}'),
            "{err}"
        );
        let (_, err) = draw(true, claude(DENIED_CALL));
        assert!(
            err.contains('\u{23fa}'),
            "no result: drawn as a call: {err}"
        );
    }

    #[test]
    fn plain_prints_only_the_answer_with_a_break_after_a_tool() {
        let (out, err) = draw(
            false,
            vec![
                Event::ThinkingStart,
                Event::Thinking("secret thought".into()),
                Event::Text("Looking".into()),
                Event::Tool {
                    id: None,
                    name: "Read".into(),
                    detail: "/work/a".into(),
                },
                Event::Text("Done.".into()),
                Event::Done(Done {
                    duration_ms: Some(10),
                    turns: Some(2),
                    ..Done::default()
                }),
            ],
        );
        assert_eq!(out, "Looking\nDone.\n");
        assert_eq!(err, "");
    }

    #[test]
    fn a_long_path_inside_the_folder_is_shortened_before_it_is_cut() {
        let deep = format!("/work/{}/file.rs", "d".repeat(90));
        let (_, err) = draw(
            true,
            vec![Event::Tool {
                id: None,
                name: "Read".into(),
                detail: deep,
            }],
        );
        let err = console::strip_ansi_codes(&err).into_owned();
        assert!(
            err.contains(&format!("Read {}/file.rs\n", "d".repeat(90))),
            "{err:?}"
        );
        assert!(!err.contains('\u{2026}'), "{err:?}");
        // Outside the folder the path stays whole, up to 100 characters.
        let (_, err) = draw(
            true,
            vec![Event::Tool {
                id: None,
                name: "Read".into(),
                detail: format!("/other/{}", "e".repeat(120)),
            }],
        );
        assert!(console::strip_ansi_codes(&err).contains('\u{2026}'));
    }

    #[test]
    fn fancy_draws_thinking_tools_and_the_footer_on_stderr() {
        let (out, err) = draw(
            true,
            vec![
                Event::ThinkingStart,
                Event::Thinking("a thought".into()),
                Event::Tool {
                    id: None,
                    name: "Read".into(),
                    detail: "/work/README.md".into(),
                },
                Event::Text("Answer".into()),
                Event::Done(Done {
                    duration_ms: Some(1234),
                    turns: Some(3),
                    cost_usd: Some(0.0123),
                    ..Done::default()
                }),
            ],
        );
        assert_eq!(out, "Answer\n");
        for want in [
            "thinking",
            "a thought",
            "\u{23fa} Read README.md",
            "\u{2502} a thought",
            "haiku",
            "AI answer",
            "1.2s",
            "3 turns",
            "$0.0123",
        ] {
            assert!(
                console::strip_ansi_codes(&err).contains(want),
                "{want} in {err:?}"
            );
        }
    }

    #[test]
    fn an_agent_with_no_pieces_still_prints_its_answer_once() {
        let (out, _) = draw(
            false,
            vec![Event::Done(Done {
                answer: Some("whole".into()),
                ..Done::default()
            })],
        );
        assert_eq!(out, "whole\n");
    }

    #[test]
    fn a_failure_is_kept_for_the_caller() {
        let mut r = Renderer::new(Vec::new(), Vec::new(), Look::default(), None);
        r.draw(Event::Done(Done {
            error: Some("Not logged in".into()),
            ..Done::default()
        }));
        assert_eq!(r.failure(), Some("Not logged in"));
    }

    /// A run drawn live: one `look` with the live view on, events in, `(stdout, stderr)` out.
    fn live(events: Vec<Event>) -> (String, String) {
        let look = Look {
            fancy: true,
            thinking: Some(Thinking::Show),
            cwd: Some("/work".into()),
            model: Some("haiku".into()),
            rows: true,
            made: Some("AI-built"),
            summary: true,
        };
        let mut r = Renderer::new(Vec::new(), Vec::new(), look, None);
        for e in events {
            r.draw(e);
        }
        r.finish();
        (
            String::from_utf8(r.out).unwrap_or_default(),
            console::strip_ansi_codes(&String::from_utf8(r.err).unwrap_or_default()).into_owned(),
        )
    }

    fn tool(id: &str, name: &str, detail: &str) -> Event {
        Event::Tool {
            id: Some(id.into()),
            name: name.into(),
            detail: detail.into(),
        }
    }

    #[test]
    fn a_live_row_is_a_check_mark_the_tool_and_its_target_cut_to_the_width() {
        assert_eq!(
            live_row("Write", "src/app.py", None, 80),
            "\u{2713}  Write src/app.py"
        );
        let cut = live_row("Write", &"x".repeat(200), None, 30);
        assert_eq!(console::measure_text_width(&cut), 30, "{cut}");
        assert!(cut.ends_with('\u{2026}'));
        let denied = live_row("Bash", "git push", Some("not allowed"), 80);
        assert_eq!(
            denied,
            "\u{2298} Bash git push \u{2014} denied, did not run: not allowed"
        );
    }

    #[test]
    fn the_summary_is_markdown_for_a_terminal_behind_the_rail() {
        let lines = summary_lines(
            "## What I built\n\nA **watcher** with `src/app.py`:\n\n- one\n- two\n\n```sh\npython3 src/app.py\n```\n",
            60,
        );
        let text = console::strip_ansi_codes(&lines.join("\n")).into_owned();
        assert!(
            !text.contains("##") && !text.contains("**") && !text.contains('`'),
            "{text}"
        );
        assert!(
            text.contains("What I built") && text.contains("\u{2022} one"),
            "{text}"
        );
        assert!(text.contains("python3 src/app.py"), "{text}");
        assert!(
            lines.iter().all(|l| l.starts_with('\u{2502}')),
            "every line sits behind the rail: {text}"
        );
        assert!(
            lines.iter().all(|l| console::measure_text_width(l) <= 60),
            "no line is wider than the terminal: {text}"
        );
    }

    #[test]
    fn the_live_view_draws_check_rows_then_the_summary_once_and_keeps_stdout_empty() {
        let (out, err) = live(vec![
            Event::Text("Let me look first.".into()),
            tool("t1", "Read", "/work/README.md"),
            Event::ToolResult {
                id: "t1".into(),
                denied: None,
            },
            tool("t2", "Bash", "git push"),
            Event::ToolResult {
                id: "t2".into(),
                denied: Some("not allowed".into()),
            },
            Event::Text("## Done\n\nIt **works**.".into()),
            Event::Done(Done {
                duration_ms: Some(4200),
                turns: Some(3),
                ..Done::default()
            }),
        ]);
        assert_eq!(out, "", "the answer is on stderr in the live view");
        assert!(err.contains("\u{2713}  Read README.md"), "{err}");
        assert!(err.contains("\u{2298} Bash git push"), "{err}");
        assert!(
            !err.contains("Let me look first"),
            "narration is not the summary: {err}"
        );
        assert_eq!(err.matches("Done").count(), 1, "{err}");
        assert!(err.contains("It works."), "{err}");
        assert!(
            err.contains("haiku \u{b7} 4.2s \u{b7} 3 turns \u{b7} AI-built"),
            "{err}"
        );
        assert!(err.find("Read README.md") < err.find("It works."), "{err}");
    }

    #[test]
    fn a_live_run_that_ends_without_a_result_still_draws_what_it_held() {
        let (_, err) = live(vec![Event::Text("It **ends** here.".into())]);
        assert!(err.contains("It ends here."), "{err}");
    }

    #[test]
    fn a_live_failure_is_kept_for_the_caller_and_draws_no_summary() {
        let look = Look {
            fancy: true,
            summary: true,
            ..Look::default()
        };
        let mut r = Renderer::new(Vec::new(), Vec::new(), look, None);
        r.draw(Event::Done(Done {
            error: Some("boom".into()),
            ..Done::default()
        }));
        r.finish();
        assert_eq!(r.failure(), Some("boom"));
        assert_eq!(String::from_utf8(r.err).unwrap_or_default(), "");
    }
}
