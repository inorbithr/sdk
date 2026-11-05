//! The steps of `iohr project init` at a terminal, as a small machine: which step is on screen,
//! where "Back" goes, and what the person has answered so far, so a step revisited starts on
//! its earlier answer. Pure (no terminal), so every move is tested; the questions themselves
//! are drawn by `commands/project.rs` through the `*_step` functions of [`crate::ui`].
//!
//! The steps run in order: name, template, describe (only for "Describe it, your agent builds
//! it"), the template's questions, folder. After the last one the plan is shown and the person
//! creates it, changes a step (a jump, then forward again with the other answers kept) or
//! cancels. Nothing is written before "Create it".

use super::folder::{Kind, Proposal};
use crate::template::Answers;

/// One step of the wizard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Stage {
    Name,
    Template,
    Describe,
    Questions,
    Folder,
}

impl Stage {
    /// Every step, in the order they run.
    pub(crate) const ALL: [Self; 5] = [
        Self::Name,
        Self::Template,
        Self::Describe,
        Self::Questions,
        Self::Folder,
    ];

    /// The step as a person reads it, on the "Change a step" menu.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Template => "Template",
            Self::Describe => "Description",
            Self::Questions => "Questions",
            Self::Folder => "Folder",
        }
    }

    /// The step's value on a menu.
    pub(crate) const fn key(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Template => "template",
            Self::Describe => "describe",
            Self::Questions => "questions",
            Self::Folder => "folder",
        }
    }

    /// The step a menu value names.
    pub(crate) fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.key() == key)
    }

    fn after(self) -> Option<Self> {
        Self::ALL
            .get(Self::ALL.iter().position(|s| *s == self)? + 1)
            .copied()
    }

    fn before(self) -> Option<Self> {
        let i = Self::ALL.iter().position(|s| *s == self)?;
        i.checked_sub(1).and_then(|j| Self::ALL.get(j).copied())
    }
}

/// How a step ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Went {
    /// Answered (or given by a flag): on to the next step.
    Next,
    /// The person asked for the step before.
    Back,
    /// Nothing was asked here (a flag gave the answer, or there is no question): keep going the
    /// way the person was going.
    Skip,
}

/// Where the wizard stands. `at` is `None` once the last step is done (the plan comes next).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Walk {
    at: Option<Stage>,
    backwards: bool,
}

impl Walk {
    /// At the first step, going forward.
    pub(crate) const fn new() -> Self {
        Self {
            at: Some(Stage::Name),
            backwards: false,
        }
    }

    /// The step to run, or `None` when every step is done.
    pub(crate) const fn at(self) -> Option<Stage> {
        self.at
    }

    /// The nearest step before this one that asks something (`asks` says which do).
    fn earlier(self, asks: &dyn Fn(Stage) -> bool) -> Option<Stage> {
        let mut s = match self.at {
            Some(s) => s.before(),
            None => Stage::ALL.last().copied(),
        };
        while let Some(x) = s {
            if asks(x) {
                return Some(x);
            }
            s = x.before();
        }
        None
    }

    /// Whether the step on screen may offer "Back": some step before it asks something.
    pub(crate) fn can_back(self, asks: &dyn Fn(Stage) -> bool) -> bool {
        self.earlier(asks).is_some()
    }

    /// Moves on after a step ended. Forward visits every step (one that has its answer from a
    /// flag skips itself); backward lands only on steps that ask something.
    pub(crate) fn apply(&mut self, went: Went, asks: &dyn Fn(Stage) -> bool) {
        let back = match went {
            Went::Next => false,
            Went::Back => true,
            Went::Skip => self.backwards,
        };
        if back {
            if let Some(s) = self.earlier(asks) {
                self.at = Some(s);
                self.backwards = true;
                return;
            }
            // Nothing asks before this: a skipped step turns round, "Back" stays put.
            if went == Went::Back {
                return;
            }
        }
        self.backwards = false;
        self.at = self.at.and_then(Stage::after);
    }

    /// Jumps to a step (from the review's "Change a step"); the walk then runs forward.
    pub(crate) const fn jump(&mut self, to: Stage) {
        self.at = Some(to);
        self.backwards = false;
    }
}

/// What the person has answered, kept while they move between steps.
#[derive(Debug, Clone, Default)]
pub(crate) struct Draft {
    pub(crate) name: Option<String>,
    /// The menu's pick: a template name or the "describe it" entry.
    pub(crate) pick: Option<String>,
    pub(crate) describe: Option<String>,
    /// What `--describe` said, which a described project keeps whatever the menu says.
    flag_describe: Option<String>,
    /// The template's questions, as last answered.
    pub(crate) answers: Answers,
    pub(crate) folder: Option<Proposal>,
    /// Whether the agent step was confirmed, kept so the question is asked once.
    pub(crate) agent_ok: Option<bool>,
}

impl Draft {
    /// The answers the flags gave.
    pub(crate) fn from_flags(
        name: Option<String>,
        pick: Option<String>,
        describe: Option<String>,
    ) -> Self {
        Self {
            name,
            pick,
            flag_describe: describe.clone(),
            describe,
            ..Self::default()
        }
    }

    /// A new name. The folder proposals are made from it, so a proposal is dropped with the old
    /// name; a path the person typed does not depend on it and stays.
    pub(crate) fn set_name(&mut self, name: String) {
        if self.name.as_deref() != Some(name.as_str())
            && self.folder.as_ref().is_some_and(|f| f.kind != Kind::Given)
        {
            self.folder = None;
        }
        self.name = Some(name);
    }

    /// A new pick. Another template has other questions, so those answers go; a description is
    /// kept only for the "describe it" entry (a template pick falls back to what `--describe`
    /// said, as before there was a way back).
    pub(crate) fn set_pick(&mut self, pick: String, described: bool) {
        if self.pick.as_deref() != Some(pick.as_str()) {
            self.answers.clear();
            self.agent_ok = None;
        }
        if !described {
            self.describe.clone_from(&self.flag_describe);
        }
        self.pick = Some(pick);
    }
}

/// Going back from question `at` (0-based, in the order they were asked) of a template's
/// questions: `None` leaves the step; `Some(n)` keeps the answers to the first `n` questions
/// and asks question `n` again, with its earlier answer as the default.
pub(crate) const fn back_in_questions(at: usize) -> Option<usize> {
    at.checked_sub(1)
}

#[cfg(test)]
mod tests {
    use super::{Draft, Stage, Walk, Went, back_in_questions};
    use crate::project::folder::{Kind, Proposal};
    use crate::template::Answer;
    use std::path::PathBuf;

    fn everything(_: Stage) -> bool {
        true
    }

    fn at(stage: Stage) -> Walk {
        let mut w = Walk::new();
        w.jump(stage);
        w
    }

    fn folder(kind: Kind) -> Proposal {
        Proposal {
            kind,
            dir: PathBuf::from("/x/shop"),
            shown: "./shop".into(),
            reason: String::new(),
            busy: false,
        }
    }

    #[test]
    fn forward_visits_every_step_then_ends() {
        let mut w = Walk::new();
        let mut seen = vec![];
        while let Some(s) = w.at() {
            seen.push(s);
            w.apply(Went::Next, &everything);
        }
        assert_eq!(seen, Stage::ALL);
        assert_eq!(w.at(), None);
    }

    #[test]
    fn back_goes_one_step_from_each_step() {
        for (from, to) in [
            (Stage::Template, Stage::Name),
            (Stage::Describe, Stage::Template),
            (Stage::Questions, Stage::Describe),
            (Stage::Folder, Stage::Questions),
        ] {
            let mut w = at(from);
            assert!(w.can_back(&everything), "{from:?}");
            w.apply(Went::Back, &everything);
            assert_eq!(w.at(), Some(to), "back from {from:?}");
        }
    }

    #[test]
    fn the_first_step_has_no_back_and_stays_put() {
        let mut w = Walk::new();
        assert!(!w.can_back(&everything));
        w.apply(Went::Back, &everything);
        assert_eq!(w.at(), Some(Stage::Name));
    }

    #[test]
    fn back_skips_steps_that_ask_nothing() {
        // The name came from an argument and there is no description: from the questions,
        // back is the template menu.
        let asks = |s: Stage| !matches!(s, Stage::Name | Stage::Describe);
        let mut w = at(Stage::Questions);
        assert!(w.can_back(&asks));
        w.apply(Went::Back, &asks);
        assert_eq!(w.at(), Some(Stage::Template));
        // And the template menu is now the first one that asks: no back from it.
        assert!(!w.can_back(&asks));
    }

    #[test]
    fn a_skipped_step_keeps_the_direction() {
        let asks = |s: Stage| s != Stage::Describe;
        let mut w = at(Stage::Questions);
        w.apply(Went::Back, &asks); // to the template menu, past the description
        assert_eq!(w.at(), Some(Stage::Template));
        w.apply(Went::Next, &asks);
        assert_eq!(w.at(), Some(Stage::Describe));
        w.apply(Went::Skip, &asks); // forward, so on
        assert_eq!(w.at(), Some(Stage::Questions));
        w.apply(Went::Back, &asks);
        w.apply(Went::Back, &asks);
        assert_eq!(w.at(), Some(Stage::Name));
        // A step with nothing to ask, reached going back with nothing before it, turns round.
        let only_late = |s: Stage| matches!(s, Stage::Folder);
        let mut w = at(Stage::Folder);
        w.apply(Went::Back, &only_late);
        assert_eq!(w.at(), Some(Stage::Folder), "no earlier step asks: stay");
    }

    #[test]
    fn a_skip_while_going_back_turns_round_at_the_front() {
        let asks = |s: Stage| s == Stage::Folder;
        let mut w = Walk::new();
        // Pretend the walk came back to the questions, which ask nothing.
        w.jump(Stage::Folder);
        w.apply(Went::Back, &|s| s == Stage::Questions || s == Stage::Folder);
        assert_eq!(w.at(), Some(Stage::Questions));
        w.apply(Went::Skip, &asks);
        assert_eq!(
            w.at(),
            Some(Stage::Folder),
            "nothing earlier asks: forward again"
        );
    }

    #[test]
    fn back_from_the_end_is_the_last_step() {
        let mut w = Walk::new();
        for _ in 0..5 {
            w.apply(Went::Next, &everything);
        }
        assert_eq!(w.at(), None);
        assert!(w.can_back(&everything));
        w.apply(Went::Back, &everything);
        assert_eq!(w.at(), Some(Stage::Folder));
    }

    #[test]
    fn changing_a_step_jumps_there_and_runs_forward_to_the_end() {
        let mut w = Walk::new();
        w.jump(Stage::Template);
        let mut seen = vec![];
        while let Some(s) = w.at() {
            seen.push(s);
            w.apply(Went::Next, &everything);
        }
        assert_eq!(
            seen,
            [
                Stage::Template,
                Stage::Describe,
                Stage::Questions,
                Stage::Folder
            ]
        );
    }

    #[test]
    fn menu_keys_round_trip() {
        for s in Stage::ALL {
            assert_eq!(Stage::from_key(s.key()), Some(s));
        }
        assert_eq!(Stage::from_key("back"), None);
    }

    #[test]
    fn changing_one_answer_keeps_the_others() {
        let mut d = Draft::from_flags(None, None, None);
        d.set_name("shop".into());
        d.set_pick("demo".into(), false);
        d.answers
            .insert("language".into(), Answer::Text("rust".into()));
        d.folder = Some(folder(Kind::Here));
        // The same name again: nothing is lost.
        d.set_name("shop".into());
        assert!(d.folder.is_some());
        // The same template again: its answers stay.
        d.set_pick("demo".into(), false);
        assert_eq!(d.answers.len(), 1);
        // Another folder-independent change: a description does not touch the rest.
        d.describe = Some("a shop".into());
        d.set_name("shop".into());
        assert_eq!(d.name.as_deref(), Some("shop"));
        assert_eq!(d.describe.as_deref(), Some("a shop"));
        assert_eq!(d.pick.as_deref(), Some("demo"), "template kept");
    }

    #[test]
    fn a_new_name_drops_a_proposal_but_keeps_a_typed_path() {
        let mut d = Draft::from_flags(None, None, None);
        d.set_name("shop".into());
        d.folder = Some(folder(Kind::Sibling));
        d.set_name("store".into());
        assert!(d.folder.is_none(), "proposals are made from the name");
        d.folder = Some(folder(Kind::Given));
        d.set_name("market".into());
        assert!(
            d.folder.is_some(),
            "a typed path does not depend on the name"
        );
    }

    #[test]
    fn a_new_template_drops_its_questions_and_a_description_it_does_not_use() {
        let mut d = Draft::from_flags(None, None, Some("from the flag".into()));
        d.set_pick("@describe".into(), true);
        d.describe = Some("typed at the menu".into());
        d.answers.insert("k".into(), Answer::Flag(true));
        // Back to the menu, then a template: the typed words go, the flag's stay.
        d.set_pick("demo".into(), false);
        assert!(d.answers.is_empty());
        assert_eq!(d.describe.as_deref(), Some("from the flag"));
        // And back to "describe it": the words typed before are the default again.
        d.set_pick("@describe".into(), true);
        d.describe = Some("typed again".into());
        d.set_pick("@describe".into(), true);
        assert_eq!(d.describe.as_deref(), Some("typed again"));
    }

    #[test]
    fn the_agent_confirmation_is_kept_until_another_template_is_picked() {
        let mut d = Draft::default();
        d.set_pick("@describe".into(), true);
        d.agent_ok = Some(true);
        d.set_pick("@describe".into(), true);
        assert_eq!(d.agent_ok, Some(true));
        d.set_pick("demo".into(), false);
        assert_eq!(d.agent_ok, None);
    }

    #[test]
    fn back_inside_the_questions_steps_one_question_then_leaves() {
        assert_eq!(back_in_questions(0), None);
        assert_eq!(back_in_questions(1), Some(0));
        assert_eq!(back_in_questions(3), Some(2));
    }
}
