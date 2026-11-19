//! The runner: the solver over the cases, every answer scored.

use crate::dataset::Case;
use crate::scorers::SCORERS;
use crate::solver::Solver;
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// One case, graded. `error` is empty unless the solver failed.
#[derive(Debug, Clone)]
pub struct CaseResult {
    pub id: String,
    pub answer: String,
    pub scores: BTreeMap<&'static str, u8>,
    pub error: String,
}

impl CaseResult {
    pub fn passed(&self) -> bool {
        self.error.is_empty() && self.scores.values().all(|s| *s == 1)
    }
}

/// Every result and which solver made them.
#[derive(Debug, Clone)]
pub struct Report {
    pub solver: String,
    pub stub: bool,
    pub results: Vec<CaseResult>,
}

impl Report {
    pub fn passes(&self) -> usize {
        self.results.iter().filter(|r| r.passed()).count()
    }

    pub fn pass_rate(&self) -> f64 {
        self.passes() as f64 / self.results.len() as f64
    }

    pub fn errors(&self) -> usize {
        self.results.iter().filter(|r| !r.error.is_empty()).count()
    }

    pub fn mean(&self, scorer: &str) -> f64 {
        let sum: u32 = self
            .results
            .iter()
            .map(|r| u32::from(r.scores.get(scorer).copied().unwrap_or(0)))
            .sum();
        f64::from(sum) / self.results.len() as f64
    }
}

/// Calls the solver and turns a panic into an error, so a broken solver is an error case.
fn solve(solver: &dyn Solver, case: &Case) -> Result<String, String> {
    match catch_unwind(AssertUnwindSafe(|| solver.solve(case))) {
        Ok(result) => result,
        Err(_) => Err("panic".to_string()),
    }
}

/// Runs the solver over the cases and scores each answer. A solver that fails is an error case
/// with every score 0, never a pass. The error kept is a short word, never the message: a
/// provider's message can echo the input.
pub fn evaluate(cases: &[Case], solver: &dyn Solver) -> Report {
    let mut results = Vec::new();
    for case in cases {
        let outcome = solve(solver, case);
        let mut scores = BTreeMap::new();
        for (name, scorer) in SCORERS {
            let score = match &outcome {
                Ok(answer) => scorer(case, answer),
                Err(_) => 0,
            };
            scores.insert(name, score);
        }
        let (answer, error) = match outcome {
            Ok(answer) => (answer, String::new()),
            Err(_) => (String::new(), "SolverError".to_string()),
        };
        results.push(CaseResult {
            id: case.id.clone(),
            answer,
            scores,
            error,
        });
    }
    Report {
        solver: solver.name(),
        stub: solver.stub(),
        results,
    }
}
