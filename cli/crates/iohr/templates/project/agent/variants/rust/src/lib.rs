//! The loop. One model, a set of tools, and three limits you can read in one place:
//!
//! - a budget: steps, tool calls and tokens; a run that would pass one stops and says which;
//! - an allow-list: a tool that is not on it is refused, and its code never runs;
//! - approval: a tool marked `needs_approval` runs only when someone answers yes.
//!
//! Every stop has a name (`Run::stop`), so a caller never has to guess why a run ended. The log
//! holds what happened (tool names, outcomes, counts), never a tool's arguments or results:
//! those can be customer data.

pub mod model;
pub mod tools;

use std::collections::{HashMap, HashSet};
use std::io::Write;

/// Why a run stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    Done,
    BudgetSteps,
    BudgetToolCalls,
    BudgetTokens,
    AwaitingApproval,
}

impl Stop {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Stop::Done => "done",
            Stop::BudgetSteps => "budget_steps",
            Stop::BudgetToolCalls => "budget_tool_calls",
            Stop::BudgetTokens => "budget_tokens",
            Stop::AwaitingApproval => "awaiting_approval",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Budget {
    pub max_steps: u32,
    pub max_tool_calls: u32,
    pub max_tokens: u32,
}

impl Default for Budget {
    fn default() -> Self {
        Budget { max_steps: 8, max_tool_calls: 4, max_tokens: 4000 }
    }
}

/// A tool argument: a number or a text. Parse what you need in the tool.
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    Int(i64),
    Str(String),
}

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub name: String,
    pub args: HashMap<String, Arg>,
}

/// What the model answered: text (it is done) or a tool call, and the tokens it used.
#[derive(Debug, Clone, Default)]
pub struct Turn {
    pub text: Option<String>,
    pub call: Option<ToolCall>,
    pub tokens: u32,
}

/// Anything with `next(history) -> Turn`. Wrap your provider here, signed in the way your
/// provider's subscription allows; this template never holds a key.
pub trait Model {
    fn next(&mut self, history: &[Message]) -> Turn;
}

/// What a tool does: it takes the arguments and answers with a text or an error.
pub type ToolFn = Box<dyn Fn(&HashMap<String, Arg>) -> Result<String, String>>;

pub struct Tool {
    pub name: &'static str,
    pub run: ToolFn,
    pub needs_approval: bool,
}

/// One entry of the history: a user task, an assistant answer or a tool outcome.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    User(String),
    Assistant(String),
    ToolResult { name: String, result: String },
    ToolError { name: String, error: &'static str },
}

#[derive(Debug)]
pub struct Run {
    pub stop: Stop,
    pub text: Option<String>,
    pub steps: u32,
    pub tool_calls: u32,
    pub tokens: u32,
    pub history: Vec<Message>,
}

/// The answer of an approval callback: yes runs the tool, no refuses it, `None` (no answer)
/// stops the run.
pub type Approve = Box<dyn Fn(&ToolCall) -> Option<bool>>;

pub struct Agent<'a> {
    pub model: Box<dyn Model + 'a>,
    pub tools: Vec<Tool>,
    pub allow: HashSet<&'static str>,
    pub budget: Budget,
    pub approve: Option<Approve>,
    pub log: Option<Box<dyn Write + 'a>>,
}

impl Agent<'_> {
    fn log(&mut self, line: &str) {
        if let Some(w) = self.log.as_mut() {
            let _ = writeln!(w, "{line}");
        }
    }

    pub fn run(&mut self, task: &str) -> Run {
        let mut history = vec![Message::User(task.to_owned())];
        let (mut steps, mut tool_calls, mut tokens) = (0u32, 0u32, 0u32);
        let finish = |me: &mut Self, stop: Stop, text: Option<String>, history, steps, tool_calls, tokens| {
            me.log(&format!(
                r#"{{"event":"stop","reason":"{}","steps":{steps},"tool_calls":{tool_calls},"tokens":{tokens}}}"#,
                stop.name()
            ));
            Run { stop, text, steps, tool_calls, tokens, history }
        };
        loop {
            if steps >= self.budget.max_steps {
                return finish(self, Stop::BudgetSteps, None, history, steps, tool_calls, tokens);
            }
            let turn = self.model.next(&history);
            steps += 1;
            if tokens + turn.tokens > self.budget.max_tokens {
                // this turn is dropped: it is over the budget
                return finish(self, Stop::BudgetTokens, None, history, steps, tool_calls, tokens);
            }
            tokens += turn.tokens;
            let Some(call) = turn.call else {
                let text = turn.text.unwrap_or_default();
                history.push(Message::Assistant(text.clone()));
                return finish(self, Stop::Done, Some(text), history, steps, tool_calls, tokens);
            };
            if tool_calls >= self.budget.max_tool_calls {
                return finish(self, Stop::BudgetToolCalls, None, history, steps, tool_calls, tokens);
            }
            let name: String = call.name.chars().take(64).collect();
            let Some(index) = self
                .tools
                .iter()
                .position(|t| t.name == call.name && self.allow.contains(t.name))
            else {
                self.log(&format!(r#"{{"event":"refused","tool":"{name}","why":"not_allowed"}}"#));
                history.push(Message::ToolError { name: call.name, error: "not_allowed" });
                continue;
            };
            if self.tools[index].needs_approval {
                let answer = self.approve.as_ref().and_then(|f| f(&call));
                match answer {
                    None => {
                        self.log(&format!(r#"{{"event":"approval","tool":"{name}","answer":"none"}}"#));
                        return finish(self, Stop::AwaitingApproval, None, history, steps, tool_calls, tokens);
                    }
                    Some(false) => {
                        self.log(&format!(r#"{{"event":"approval","tool":"{name}","answer":"no"}}"#));
                        history.push(Message::ToolError { name: call.name, error: "denied" });
                        continue;
                    }
                    Some(true) => {
                        self.log(&format!(r#"{{"event":"approval","tool":"{name}","answer":"yes"}}"#));
                    }
                }
            }
            let outcome = (self.tools[index].run)(&call.args);
            tool_calls += 1;
            match outcome {
                Ok(result) => {
                    self.log(&format!(r#"{{"event":"tool","tool":"{name}","ok":true}}"#));
                    history.push(Message::ToolResult { name: call.name, result });
                }
                Err(_) => {
                    self.log(&format!(r#"{{"event":"tool","tool":"{name}","ok":false}}"#));
                    history.push(Message::ToolError { name: call.name, error: "failed" });
                }
            }
        }
    }
}
