//! The checks a schema cannot express, on top of the schema itself.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde_json::Value;

use super::board::{Board, Card, schema_problems};
use super::parse_date;

/// Every problem with a board, as one line each; empty when it is valid.
///
/// The schema runs first; when it fails, only its errors are given. Then the checks a schema
/// cannot express: unique ids, real dates, `start` on or before `due`, each `depends_on` names
/// another card that exists and starts no later, no dependency cycles, and (when `atlas` is
/// given) every `docs` path exists under it.
pub(crate) fn check(value: &Value, atlas: Option<&Path>) -> Vec<String> {
    let schema = schema_problems(value);
    if !schema.is_empty() {
        return schema;
    }
    match serde_json::from_value::<Board>(value.clone()) {
        Ok(b) => check_board(&b, atlas),
        Err(e) => vec![format!("board: {e}")],
    }
}

/// The checks of [`check`] that a schema cannot express, on a typed board.
pub(crate) fn check_board(b: &Board, atlas: Option<&Path>) -> Vec<String> {
    let mut out = Vec::new();
    if parse_date(&b.updated).is_none() {
        out.push(format!("updated {} is not a date (YYYY-MM-DD)", b.updated));
    }
    let mut seen = HashSet::new();
    for c in &b.cards {
        if !seen.insert(c.id.as_str()) {
            out.push(format!("card id {} is used twice", c.id));
        }
    }
    let by_id: HashMap<&str, &Card> = b.cards.iter().map(|c| (c.id.as_str(), c)).collect();
    for c in &b.cards {
        let id = &c.id;
        let start = parse_date(&c.start);
        let due = parse_date(&c.due);
        if start.is_none() {
            out.push(format!("{id}: start {} is not a date (YYYY-MM-DD)", c.start));
        }
        if due.is_none() {
            out.push(format!("{id}: due {} is not a date (YYYY-MM-DD)", c.due));
        }
        if let (Some(s), Some(d)) = (start, due)
            && s > d
        {
            out.push(format!("{id}: start {} is after due {}", c.start, c.due));
        }
        for dep in &c.depends_on {
            if dep == id {
                out.push(format!("{id} depends on itself"));
                continue;
            }
            match by_id.get(dep.as_str()) {
                None => out.push(format!("{id} depends on {dep}, which is not on the board")),
                Some(d) => {
                    if let (Some(ds), Some(cs)) = (parse_date(&d.start), start)
                        && ds > cs
                    {
                        out.push(format!(
                            "{id} starts {} before {dep}, which it depends on, starts {}",
                            c.start, d.start
                        ));
                    }
                }
            }
        }
        if let Some(a) = atlas {
            for p in c.docs.all() {
                if !a.join(p).is_file() {
                    out.push(format!("{id}: docs path {p} does not exist under atlas/"));
                }
            }
        }
    }
    out.extend(cycles(b));
    out
}

/// Dependency cycles, each once, as "cycle: a -> b -> a" ("a depends on b, b on a").
fn cycles(b: &Board) -> Vec<String> {
    struct Walk<'a> {
        b: &'a Board,
        idx: HashMap<&'a str, usize>,
        colour: Vec<u8>,
        stack: Vec<usize>,
        out: Vec<String>,
    }
    impl Walk<'_> {
        fn visit(&mut self, i: usize) {
            self.colour[i] = 1;
            self.stack.push(i);
            let card = &self.b.cards[i];
            for dep in &card.depends_on {
                if *dep == card.id {
                    continue;
                }
                let Some(&j) = self.idx.get(dep.as_str()) else {
                    continue;
                };
                match self.colour[j] {
                    0 => self.visit(j),
                    1 => {
                        let from = self.stack.iter().position(|&k| k == j).unwrap_or(0);
                        let mut names: Vec<&str> = self.stack[from..]
                            .iter()
                            .map(|&k| self.b.cards[k].id.as_str())
                            .collect();
                        names.push(self.b.cards[j].id.as_str());
                        self.out.push(format!("cycle: {}", names.join(" -> ")));
                    }
                    _ => {}
                }
            }
            self.stack.pop();
            self.colour[i] = 2;
        }
    }
    let mut idx = HashMap::new();
    for (i, c) in b.cards.iter().enumerate() {
        idx.entry(c.id.as_str()).or_insert(i);
    }
    let mut w = Walk {
        b,
        idx,
        colour: vec![0; b.cards.len()],
        stack: Vec::new(),
        out: Vec::new(),
    };
    for i in 0..b.cards.len() {
        if w.colour[i] == 0 {
            w.visit(i);
        }
    }
    w.out
}
