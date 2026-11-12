//! The passes-on-init test: a scripted run within budget; a run that would pass a budget stops
//! with a named reason; a tool outside the allow-list is refused and never runs; a tool that
//! needs approval does not run without an answer.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use @@MODULE@@::model::{ScriptedModel, demo};
use @@MODULE@@::tools::{allow, tools};
use @@MODULE@@::{Agent, Approve, Arg, Budget, Message, Stop, Tool, ToolCall, Turn};

fn call(name: &str, args: &[(&str, Arg)], tokens: u32) -> Turn {
    let args = args.iter().map(|(k, v)| ((*k).to_owned(), v.clone())).collect::<HashMap<_, _>>();
    Turn { call: Some(ToolCall { name: name.into(), args }), tokens, text: None }
}

fn say(text: &str) -> Turn {
    Turn { text: Some(text.into()), ..Turn::default() }
}

fn agent(turns: Vec<Turn>, tools: Vec<Tool>, allow: HashSet<&'static str>, budget: Budget) -> Agent<'static> {
    Agent { model: Box::new(ScriptedModel::new(turns)), tools, allow, budget, approve: None, log: None }
}

fn add_turn(tokens: u32) -> Turn {
    call("add", &[("a", Arg::Int(1)), ("b", Arg::Int(1))], tokens)
}

#[test]
fn a_scripted_run_finishes_within_the_budget() {
    let mut a = Agent { model: Box::new(demo()), tools: tools(), allow: allow(), budget: Budget::default(), approve: None, log: None };
    let run = a.run("add 2 and 3");
    assert_eq!((run.stop, run.text.as_deref(), run.tool_calls), (Stop::Done, Some("2 + 3 = 5"), 1));
    assert!(run.steps <= Budget::default().max_steps);
    assert_eq!(run.history[1], Message::ToolResult { name: "add".into(), result: "5".into() });
}

#[test]
fn a_run_that_would_pass_a_budget_stops_and_names_it() {
    let forever = || (0..50).map(|_| add_turn(10)).collect::<Vec<_>>();
    let b = |s, c, t| Budget { max_steps: s, max_tool_calls: c, max_tokens: t };
    assert_eq!(agent(forever(), tools(), allow(), b(8, 3, 4000)).run("t").stop, Stop::BudgetToolCalls);
    assert_eq!(agent(forever(), tools(), allow(), b(2, 9, 4000)).run("t").stop, Stop::BudgetSteps);
    let heavy = (0..5).map(|_| add_turn(3000)).collect::<Vec<_>>();
    let run = agent(heavy, tools(), allow(), b(8, 9, 4000)).run("t");
    assert_eq!(run.stop, Stop::BudgetTokens);
    assert!(run.tokens <= 4000);
}

#[test]
fn a_tool_outside_the_allow_list_is_refused_and_never_runs() {
    let ran = Rc::new(Cell::new(0));
    let seen = Rc::clone(&ran);
    let mut all = tools();
    all.push(Tool { name: "shell", run: Box::new(move |_| { seen.set(seen.get() + 1); Ok(String::new()) }), needs_approval: false });
    let turns = vec![call("shell", &[("cmd", Arg::Str("rm -rf /".into()))], 0), say("ok")];
    let run = agent(turns, all, allow(), Budget::default()).run("t");
    assert_eq!(ran.get(), 0);
    assert_eq!(run.history[1], Message::ToolError { name: "shell".into(), error: "not_allowed" });
    assert_eq!((run.stop, run.tool_calls), (Stop::Done, 0));
}

#[test]
fn a_tool_that_needs_approval_does_not_run_without_an_answer() {
    let ran = Rc::new(Cell::new(0));
    let make = |ran: &Rc<Cell<u32>>, answer: Option<Option<bool>>| {
        let seen = Rc::clone(ran);
        let tools = vec![Tool { name: "delete_note", run: Box::new(move |_| { seen.set(seen.get() + 1); Ok(String::new()) }), needs_approval: true }];
        let turns = vec![call("delete_note", &[("id", Arg::Str("n1".into()))], 0), say("ok")];
        let mut a = agent(turns, tools, HashSet::from(["delete_note"]), Budget::default());
        a.approve = answer.map(|r| Box::new(move |_: &ToolCall| r) as Approve);
        a
    };
    assert_eq!(make(&ran, None).run("t").stop, Stop::AwaitingApproval);
    assert_eq!(make(&ran, Some(None)).run("t").stop, Stop::AwaitingApproval);
    let denied = make(&ran, Some(Some(false))).run("t");
    assert_eq!(denied.stop, Stop::Done);
    assert_eq!(denied.history[1], Message::ToolError { name: "delete_note".into(), error: "denied" });
    assert_eq!(ran.get(), 0);
    let yes = make(&ran, Some(Some(true))).run("t");
    assert_eq!((yes.stop, ran.get()), (Stop::Done, 1));
}

struct Sink(Arc<Mutex<Vec<u8>>>);

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().map_err(|_| std::io::Error::other("poisoned"))?.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

#[test]
fn the_log_holds_what_happened_and_never_a_tool_argument() {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let turns = vec![call("add", &[("a", Arg::Int(424_242)), ("b", Arg::Int(1))], 0), say("secret-answer")];
    let mut a = agent(turns, tools(), allow(), Budget::default());
    a.log = Some(Box::new(Sink(Arc::clone(&buf))));
    a.run("t");
    let text = String::from_utf8(buf.lock().map(|b| b.clone()).unwrap_or_default()).unwrap_or_default();
    assert!(text.contains(r#""event":"tool""#), "{text}");
    assert!(!text.contains("424242") && !text.contains("secret-answer"), "{text}");
}
