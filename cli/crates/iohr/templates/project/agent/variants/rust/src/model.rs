//! A scripted model: it answers from a list, so a run is repeatable and costs nothing. It is a
//! stub, labelled as one everywhere it prints. To use a real model, write a type with
//! `next(history) -> Turn` that calls your provider and returns its text or tool call and the
//! tokens it used; the loop does not change.

use std::collections::{HashMap, VecDeque};

use crate::{Arg, Message, Model, ToolCall, Turn};

pub const STUB: &str = "stub: scripted model, no real model was called";

pub struct ScriptedModel {
    turns: VecDeque<Turn>,
}

impl ScriptedModel {
    #[must_use]
    pub fn new(turns: Vec<Turn>) -> Self {
        ScriptedModel { turns: turns.into() }
    }
}

impl Model for ScriptedModel {
    fn next(&mut self, _history: &[Message]) -> Turn {
        self.turns
            .pop_front()
            .unwrap_or_else(|| Turn { text: Some("(the script has ended)".into()), ..Turn::default() })
    }
}

#[must_use]
pub fn demo() -> ScriptedModel {
    let args = HashMap::from([("a".to_owned(), Arg::Int(2)), ("b".to_owned(), Arg::Int(3))]);
    ScriptedModel::new(vec![
        Turn { call: Some(ToolCall { name: "add".into(), args }), tokens: 40, text: None },
        Turn { text: Some("2 + 3 = 5".into()), tokens: 20, call: None },
    ])
}
