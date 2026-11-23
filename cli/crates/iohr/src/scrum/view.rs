//! Reading a board: the Trello view, filtered rows, one card, what to do next, the plan by day,
//! and the README region rendered from the board.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use chrono::{Duration, NaiveDate};
use serde::Serialize;
use serde_json::Value;

use super::board::{Board, Card};
use super::{Error, LISTS, fmt_date, parse_date};
use crate::ui::Paint;

/// The README region `render` owns; everything outside it is written by hand.
pub(crate) const START: &str = "<!-- scrum:board:start -->";
pub(crate) const END: &str = "<!-- scrum:board:end -->";

/// Hours on a day above which the plan flags it when the board sets no `day_hours` (one person's day, RFC 0003).
pub(crate) const DAY_HOURS: f64 = 20.0;

/// Hours for people: `6`, `2.5`.
pub(crate) fn fmt_h(h: f64) -> String {
    // `+ 0.0` turns the -0.0 of an empty sum into 0.
    let r = (h * 10.0).round() / 10.0 + 0.0;
    format!("{r}")
}

fn hours(cards: &[&Card]) -> f64 {
    cards.iter().map(|c| c.h()).sum()
}

/// `11-14`, or `11-14 to 11-15` when the card spans days.
pub(crate) fn span(c: &Card) -> String {
    let short = |d: &str| d.get(5..).unwrap_or(d).to_string();
    if c.start == c.due { short(&c.start) } else { format!("{} to {}", short(&c.start), short(&c.due)) }
}

/// Idea numbers, with runs of three or more as `1 to 5`.
pub(crate) fn ideas(n: &[u8]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < n.len() {
        let mut j = i;
        while j + 1 < n.len() && n[j + 1] == n[j].saturating_add(1) {
            j += 1;
        }
        if j - i >= 2 {
            out.push(format!("{} to {}", n[i], n[j]));
        } else {
            out.extend(n[i..=j].iter().map(u8::to_string));
        }
        i = j + 1;
    }
    out.join(", ")
}

fn in_list<'a>(b: &'a Board, list: &str) -> Vec<&'a Card> {
    let mut v: Vec<&Card> = b.cards.iter().filter(|c| c.list == list).collect();
    v.sort_by(|x, y| x.start.cmp(&y.start));
    v
}

/// How a view is drawn: colour (a terminal, `NO_COLOR` unset) and the width lines are cut to.
/// [`Look::plain`] is for pipes, files and tests: no escapes, nothing cut. Colour comes from
/// [`Paint`], so the board follows the same rules as every other iohr view.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Look {
    pub(crate) color: bool,
    pub(crate) width: usize,
    paint: Paint,
}

impl Look {
    pub(crate) const fn new(paint: Paint, width: usize) -> Self {
        Self {
            color: paint.color(),
            width,
            paint,
        }
    }

    /// For pipes, files and tests: no escapes, nothing cut.
    pub(crate) const fn plain() -> Self {
        Self {
            color: false,
            width: usize::MAX,
            paint: Paint::plain(),
        }
    }

    /// For stdout: plain unless it is a terminal; the width is the terminal's.
    pub(crate) fn stdout(json: bool) -> Self {
        let paint = Paint::stdout(json);
        if paint.tty() {
            Self::new(paint, crate::ui::stdout_width())
        } else {
            Self::new(paint, usize::MAX)
        }
    }

    /// `s` in an SGR style such as `1` (bold), `2` (dim) or `36` (cyan), when colour is on.
    pub(crate) fn paint(&self, sgr: &str, s: &str) -> String {
        self.paint.sgr(sgr, s)
    }

    pub(crate) fn dim(&self, s: &str) -> String {
        self.paint("2", s)
    }

    /// `s` bold yellow, for what just changed (`--watch` marks moved cards).
    pub(crate) fn hot(&self, s: &str) -> String {
        self.paint("1;33", s)
    }
}


/// A card's `x.section`, empty when it has none.
pub(crate) fn section(c: &Card) -> &str {
    c.x.get("section").and_then(Value::as_str).unwrap_or_default()
}

/// A card's `x.ideas`: the idea numbers it carries, empty when it has none.
pub(crate) fn ideas_of(c: &Card) -> Vec<u8> {
    c.x.get("ideas")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|n| n.as_u64().and_then(|n| u8::try_from(n).ok()))
                .collect()
        })
        .unwrap_or_default()
}

/// Each list's colour: backlog grey, ready cyan, doing yellow, review magenta, done green.
fn list_sgr(list: &str) -> &'static str {
    match list {
        "ready" => "36",
        "doing" => "33",
        "review" => "35",
        "done" => "32",
        _ => "90",
    }
}

fn size_sgr(size: &str) -> &'static str {
    match size {
        "S" => "32",
        "M" => "33",
        _ => "31",
    }
}

/// `s` cut to `n` characters, with `…` when cut.
fn cut(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_owned();
    }
    let mut t: String = s.chars().take(n.saturating_sub(1)).collect();
    t.push('…');
    t
}

/// The longest card id, so ids line up.
fn id_width(cards: &[&Card]) -> usize {
    cards.iter().map(|c| c.id.chars().count()).max().unwrap_or(0)
}

/// The board by list, Trello style.
pub(crate) fn board_view(b: &Board, today: &str, look: &Look) -> String {
    let open: Vec<&Card> = b.cards.iter().filter(|c| c.list != "dropped").collect();
    let w = id_width(&open.iter().chain(in_list(b, "dropped").iter()).copied().collect::<Vec<_>>());
    let mut out = if look.color {
        let done: Vec<&Card> = open.iter().copied().filter(|c| c.list == "done").collect();
        format!(
            "{}  {}\n{}  {}\n",
            look.paint("1", &b.board),
            look.paint("2", &format!("updated {}", b.updated)),
            bar(done.len(), open.len(), 24, look),
            look.paint("2", &format!("{} of {} cards done, {} of {} h", done.len(), open.len(), fmt_h(hours(&done)), fmt_h(hours(&open))))
        )
    } else {
        format!("{} (updated {})\n", b.board, b.updated)
    };
    for list in LISTS {
        let cards = in_list(b, list);
        if look.color {
            let head = format!("{} {}", look.paint(list_sgr(list), "●"), look.paint(&format!("1;{}", list_sgr(list)), list));
            let count = format!("{}, {} h", n_cards(cards.len()), fmt_h(hours(&cards)));
            let _ = writeln!(out, "\n{head}  {}", look.paint("2", &count));
            let _ = writeln!(out, "{}", look.paint("2", &"─".repeat(look.width.min(100))));
        } else {
            let _ = writeln!(out, "\n{list} ({}, {} h)", n_cards(cards.len()), fmt_h(hours(&cards)));
        }
        for c in cards {
            let _ = writeln!(out, "  {}", line(c, today, w, look.width.saturating_sub(2), look));
        }
    }
    out
}

/// A progress bar `n` of `total`, `cells` wide.
fn bar(n: usize, total: usize, cells: usize, look: &Look) -> String {
    let full = (n * cells).checked_div(total).unwrap_or(0);
    let (on, off) = if look.color { ("█", "░") } else { ("#", "-") };
    format!("{}{}", look.paint("32", &on.repeat(full)), look.paint("90", &off.repeat(cells - full)))
}

fn flags(c: &Card, today: &str) -> Vec<&'static str> {
    let mut f = Vec::new();
    if c.late(today) {
        f.push("late");
    }
    if !c.blocked_by.is_empty() {
        f.push("blocked");
    }
    f
}

/// One card on one line: id, size, hours, dates, checklist, title. In colour the late and blocked
/// markers come before the title and the title is cut to `width`; plain keeps them after it.
fn line(c: &Card, today: &str, id_w: usize, width: usize, look: &Look) -> String {
    let done = c.checklist.iter().filter(|x| x.done).count();
    let ticks = format!("{done}/{}", c.checklist.len());
    let f = flags(c, today);
    if !look.color {
        let tail = if f.is_empty() { String::new() } else { format!("  [{}]", f.join(", ")) };
        return format!("{:<id_w$} {} {:>4} h  {:<14} {ticks:<4} {}{tail}", c.id, c.size, fmt_h(c.h()), span(c), c.title);
    }
    // The same text as the painted line below, to count what is left for the title.
    let head = format!("{:<id_w$} {} {:>4} h  {:<14}  {ticks:<4} ", c.id, c.size, fmt_h(c.h()), span(c));
    let marks: String = f.iter().map(|m| format!("{m} ")).collect();
    let room = width.saturating_sub(head.chars().count() + marks.chars().count()).max(12);
    let ticks_sgr = if c.checklist.is_empty() || done == 0 { "2" } else if done == c.checklist.len() { "32" } else { "33" };
    let painted_marks: String = f.iter().map(|m| look.paint(if *m == "late" { "1;31" } else { "1;33" }, m) + " ").collect();
    let title = cut(&c.title, room);
    let title = if c.list == "done" || c.list == "dropped" { look.paint("2", &title) } else { title };
    format!(
        "{} {} {}  {}  {} {painted_marks}{title}",
        look.paint("1", &format!("{:<id_w$}", c.id)),
        look.paint(size_sgr(&c.size), &c.size),
        look.paint("2", &format!("{:>4} h", fmt_h(c.h()))),
        look.paint("2", &format!("{:<14}", span(c))),
        look.paint(ticks_sgr, &format!("{ticks:<4}")),
    )
}

/// Which cards `list` shows.
#[derive(Debug, Default, Clone)]
pub(crate) struct Filter {
    pub(crate) list: Option<String>,
    pub(crate) section: Option<String>,
    pub(crate) label: Option<String>,
    pub(crate) late: bool,
    pub(crate) blocked: bool,
}

/// The cards a filter keeps, in board order.
pub(crate) fn list<'a>(b: &'a Board, f: &Filter, today: &str) -> Vec<&'a Card> {
    b.cards
        .iter()
        .filter(|c| f.list.as_ref().is_none_or(|l| &c.list == l))
        .filter(|c| f.section.as_ref().is_none_or(|s| section(c) == s))
        .filter(|c| f.label.as_ref().is_none_or(|l| c.labels.contains(l)))
        .filter(|c| !f.late || c.late(today))
        .filter(|c| !f.blocked || !c.blocked_by.is_empty())
        .collect()
}

/// Cards as rows for people.
pub(crate) fn rows(cards: &[&Card], today: &str, look: &Look) -> String {
    let mut out = String::new();
    let w = id_width(cards);
    let sw = cards.iter().map(|c| section(c).chars().count()).max().unwrap_or(0).max(7);
    for c in cards {
        let lead = format!("{:<7} {:<sw$} ", c.list, section(c));
        let rest = line(c, today, w, look.width.saturating_sub(lead.chars().count()), look);
        let lead = if look.color { format!("{} {} ", look.paint(list_sgr(&c.list), &format!("{:<7}", c.list)), look.paint("2", &format!("{:<sw$}", section(c)))) } else { lead };
        let _ = writeln!(out, "{lead}{rest}");
    }
    let _ = writeln!(out, "{}", look.paint("2", &format!("{}, {} h", n_cards(cards.len()), fmt_h(hours(cards)))));
    out
}

/// One card, every field, for people.
pub(crate) fn show(c: &Card) -> String {
    let mut out = format!("{}: {}\n", c.id, c.title);
    let _ = writeln!(out, "  list {}, section {}, size {}, {} h, {} to {}", c.list, section(c), c.size, fmt_h(c.h()), c.start, c.due);
    let mut field = |name: &str, v: &[String]| {
        if !v.is_empty() {
            let _ = writeln!(out, "  {name}: {}", v.join(", "));
        }
    };
    field("labels", &c.labels);
    field("depends on", &c.depends_on);
    field("blocked by", &c.blocked_by);
    field("docs", &c.docs.all().cloned().collect::<Vec<_>>());
    field("builds on", &c.builds_on);
    let idea_nums = ideas_of(c);
    if !idea_nums.is_empty() {
        let _ = writeln!(out, "  ideas: {}", ideas(&idea_nums));
    }
    for (k, val) in c.x.iter().filter(|(k, _)| *k != "section" && *k != "ideas") {
        let _ = writeln!(out, "  x.{k}: {val}");
    }
    if let Some(p) = &c.planner {
        let _ = writeln!(out, "  planner: {}/{}", p.plan, p.item);
    }
    if let Some(p) = &c.pr {
        let _ = writeln!(out, "  pr: {p}");
    }
    let _ = writeln!(out, "checklist");
    for (i, x) in c.checklist.iter().enumerate() {
        let _ = writeln!(out, "  {}. [{}] {}", i + 1, if x.done { "x" } else { " " }, x.text);
    }
    for (name, v) in [("acceptance", &c.acceptance), ("tests", &c.tests)] {
        let _ = writeln!(out, "{name}");
        for x in v {
            let _ = writeln!(out, "  - {x}");
        }
    }
    if let Some(n) = &c.notes {
        let _ = writeln!(out, "notes");
        for l in n.lines() {
            let _ = writeln!(out, "  {l}");
        }
    }
    out
}

/// Ready cards whose `depends_on` are all done and that nothing blocks, by due date, at most `n`.
pub(crate) fn next(b: &Board, n: usize) -> Vec<&Card> {
    let mut v: Vec<&Card> = b
        .cards
        .iter()
        .filter(|c| c.list == "ready" && c.blocked_by.is_empty())
        .filter(|c| c.depends_on.iter().all(|d| b.list_of(d) == Some("done")))
        .collect();
    v.sort_by(|x, y| (&x.due, &x.start, &x.id).cmp(&(&y.due, &y.start, &y.id)));
    v.truncate(n);
    v
}

/// Planned hours on one day.
#[derive(Debug, Serialize, PartialEq)]
pub(crate) struct Day {
    pub(crate) date: String,
    pub(crate) hours: f64,
    /// More than the plan's `day_hours`.
    pub(crate) over: bool,
}

/// A card that waits on something outside the board.
#[derive(Debug, Serialize, PartialEq)]
pub(crate) struct Blocked {
    pub(crate) id: String,
    pub(crate) by: Vec<String>,
}

/// "1 card" or "n cards".
pub(crate) fn n_cards(n: usize) -> String {
    if n == 1 { "1 card".into() } else { format!("{n} cards") }
}

/// The plan of the cards still to build (backlog, ready, doing): hours by day, late and blocked
/// cards, the longest chain. Review cards are built and wait for the owner's check, so they are
/// counted apart and never load a day or show as late.
#[derive(Debug, Serialize)]
pub(crate) struct Plan {
    /// Build hours one day holds (the board's `day_hours`, else [`DAY_HOURS`]).
    pub(crate) day_hours: f64,
    pub(crate) open_cards: usize,
    pub(crate) open_hours: f64,
    pub(crate) review_cards: usize,
    pub(crate) review_hours: f64,
    pub(crate) days: Vec<Day>,
    pub(crate) late: Vec<String>,
    pub(crate) blocked: Vec<Blocked>,
    /// The open dependency chain with the most hours, first card first.
    pub(crate) chain: Vec<String>,
    pub(crate) chain_hours: f64,
}

/// The plan of a board's cards still to build. A card's hours are spread evenly over its days.
pub(crate) fn plan(b: &Board, today: &str) -> Plan {
    let open: Vec<&Card> = b.cards.iter().filter(|c| c.to_build()).collect();
    let review: Vec<&Card> = b.cards.iter().filter(|c| c.list == "review").collect();
    let mut by_day: BTreeMap<NaiveDate, f64> = BTreeMap::new();
    for c in &open {
        if let (Some(s), Some(d)) = (parse_date(&c.start), parse_date(&c.due)) {
            let n = ((d - s).num_days() + 1).max(1);
            #[allow(clippy::cast_precision_loss)]
            let each = c.h() / n as f64;
            for i in 0..n {
                *by_day.entry(s + Duration::days(i)).or_default() += each;
            }
        }
    }
    let cap = b.day_hours.unwrap_or(DAY_HOURS);
    let days = by_day
        .into_iter()
        .map(|(d, h)| Day { date: fmt_date(d), hours: (h * 10.0).round() / 10.0, over: h > cap })
        .collect();
    let (chain, chain_hours) = longest_chain(b);
    Plan {
        day_hours: cap,
        open_cards: open.len(),
        open_hours: hours(&open),
        review_cards: review.len(),
        review_hours: hours(&review),
        days,
        late: open.iter().filter(|c| c.late(today)).map(|c| c.id.clone()).collect(),
        blocked: open
            .iter()
            .filter(|c| !c.blocked_by.is_empty())
            .map(|c| Blocked { id: c.id.clone(), by: c.blocked_by.clone() })
            .collect(),
        chain,
        chain_hours,
    }
}

/// The chain still to build with the most hours. A cycle (which `check` refuses) counts as no dependency.
fn longest_chain(b: &Board) -> (Vec<String>, f64) {
    let by_id: HashMap<&str, &Card> = b.cards.iter().filter(|c| c.to_build()).map(|c| (c.id.as_str(), c)).collect();
    let mut memo: HashMap<&str, (f64, Option<&str>)> = HashMap::new();
    fn walk<'a>(
        id: &'a str,
        by_id: &HashMap<&'a str, &'a Card>,
        memo: &mut HashMap<&'a str, (f64, Option<&'a str>)>,
        busy: &mut Vec<&'a str>,
    ) -> f64 {
        if let Some((h, _)) = memo.get(id) {
            return *h;
        }
        let Some(c) = by_id.get(id) else { return 0.0 };
        if busy.contains(&id) {
            return 0.0;
        }
        busy.push(id);
        let mut best: (f64, Option<&str>) = (0.0, None);
        for d in &c.depends_on {
            if let Some(dc) = by_id.get(d.as_str()) {
                let h = walk(dc.id.as_str(), by_id, memo, busy);
                if h > best.0 {
                    best = (h, Some(dc.id.as_str()));
                }
            }
        }
        busy.pop();
        let total = c.h() + best.0;
        memo.insert(id, (total, best.1));
        total
    }
    let mut top: (f64, Option<&str>) = (0.0, None);
    let mut busy = Vec::new();
    for c in b.cards.iter().filter(|c| c.to_build()) {
        let h = walk(c.id.as_str(), &by_id, &mut memo, &mut busy);
        if h > top.0 {
            top = (h, Some(c.id.as_str()));
        }
    }
    let mut chain = Vec::new();
    let mut at = top.1;
    while let Some(id) = at {
        if chain.iter().any(|x: &String| x == id) {
            break;
        }
        chain.push(id.to_string());
        at = memo.get(id).and_then(|(_, n)| *n);
    }
    chain.reverse();
    (chain, (top.0 * 10.0).round() / 10.0)
}

/// The plan for people.
pub(crate) fn plan_text(p: &Plan) -> String {
    let mut out = format!(
        "{} to build, {} h; {} in review, {} h (built, not counted below)\n\nhours by day (a day holds {} h)\n",
        n_cards(p.open_cards),
        fmt_h(p.open_hours),
        n_cards(p.review_cards),
        fmt_h(p.review_hours),
        fmt_h(p.day_hours)
    );
    for d in &p.days {
        let _ = writeln!(out, "  {}  {:>5} h{}", d.date, fmt_h(d.hours), if d.over { format!("  over {} h", fmt_h(p.day_hours)) } else { String::new() });
    }
    let _ = writeln!(out, "\nlate: {}", if p.late.is_empty() { "none".into() } else { p.late.join(", ") });
    let _ = writeln!(out, "blocked:{}", if p.blocked.is_empty() { " none" } else { "" });
    for x in &p.blocked {
        let _ = writeln!(out, "  {}: {}", x.id, x.by.join("; "));
    }
    let _ = writeln!(out, "longest chain ({} h): {}", fmt_h(p.chain_hours), p.chain.join(" -> "));
    out
}

fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

fn table(out: &mut String, head: &[&str], rows: Vec<Vec<String>>) {
    let _ = writeln!(out, "| {} |", head.join(" | "));
    let _ = writeln!(out, "|{}", "---|".repeat(head.len()));
    for r in rows {
        out.push('|');
        for c in r {
            out.push_str(&if c.is_empty() { " |".to_string() } else { format!(" {} |", cell(&c)) });
        }
        out.push('\n');
    }
}

/// The Planner status a list maps to (RFC 0003).
pub(crate) fn planner_status(list: &str) -> &'static str {
    match list {
        "backlog" => "proposed",
        "ready" => "ready",
        "doing" | "review" => "building",
        "done" => "done",
        _ => "dropped",
    }
}

/// The README region: the lists, then the board by list.
pub(crate) fn render(b: &Board) -> String {
    let mut out = String::from("## Lists (Trello columns)\n\n");
    let meaning = [
        ("backlog", "Not ready: blocked, or waiting on the owner"),
        ("ready", "Everything needed is known; can start"),
        ("doing", "Being built now"),
        ("review", "Built, being checked (tests, screenshots); on sync its Planner item gets the note \"in review\""),
        ("done", "Every checklist item ticked, with a PR or written evidence"),
        ("dropped", "Abandoned, with the reason in `notes`"),
    ];
    table(
        &mut out,
        &["List", "Planner status", "Meaning"],
        meaning.iter().map(|(l, m)| vec![l.to_string(), planner_status(l).into(), m.to_string()]).collect(),
    );
    let all: Vec<&Card> = b.cards.iter().collect();
    let (first, last) = (
        b.cards.iter().map(|c| c.start.as_str()).min().unwrap_or(""),
        b.cards.iter().map(|c| c.due.as_str()).max().unwrap_or(""),
    );
    let _ = write!(
        out,
        "\n## The board on {}\n\n{}, {} hours, from {first} to {last}.\n",
        b.updated,
        n_cards(all.len()),
        fmt_h(hours(&all))
    );
    let mut empty = Vec::new();
    for list in LISTS {
        let cards = in_list(b, list);
        if cards.is_empty() {
            empty.push(list);
            continue;
        }
        let _ = writeln!(out, "\n### {list}\n");
        match list {
            "ready" => table(
                &mut out,
                &["Card", "Section", "Size", "Hours", "Start to due", "Depends on", "Ideas"],
                cards
                    .iter()
                    .map(|c| {
                        vec![
                            c.id.clone(),
                            section(c).to_string(),
                            c.size.clone(),
                            fmt_h(c.h()),
                            span(c),
                            c.depends_on.join(", "),
                            ideas(&ideas_of(c)),
                        ]
                    })
                    .collect(),
            ),
            "backlog" => table(
                &mut out,
                &["Card", "Section", "Hours", "Dates", "Blocked by"],
                cards
                    .iter()
                    .map(|c| vec![c.id.clone(), section(c).to_string(), fmt_h(c.h()), span(c), c.blocked_by.join("; ")])
                    .collect(),
            ),
            _ => table(
                &mut out,
                &["Card", "Title", "Size", "Hours", "Dates", "PR"],
                cards
                    .iter()
                    .map(|c| {
                        vec![
                            c.id.clone(),
                            c.title.clone(),
                            c.size.clone(),
                            fmt_h(c.h()),
                            span(c),
                            c.pr.clone().unwrap_or_default(),
                        ]
                    })
                    .collect(),
            ),
        }
    }
    if !empty.is_empty() {
        let _ = writeln!(out, "\nEmpty: {}.", empty.join(", "));
    }
    out
}

/// The README with its region replaced by [`render`]. Refused when a marker is missing.
pub(crate) fn render_readme(old: &str, b: &Board) -> Result<String, Error> {
    let (Some(s), Some(e)) = (old.find(START), old.find(END)) else {
        return Err(Error::Refused(vec![format!("the README has no {START} ... {END} region to render into")]));
    };
    if e < s {
        return Err(Error::Refused(vec![format!("{END} comes before {START} in the README")]));
    }
    Ok(format!("{}{START}\n\n{}\n{}", &old[..s], render(b), &old[e..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scrum::board::tests::{board, card};

    #[test]
    fn next_plan_and_ideas() {
        let mut a = card("aa", "done", &[]);
        a.hours = 3.into();
        let mut c = card("cc", "ready", &["bb"]);
        c.hours = 4.into();
        let mut d = card("dd", "ready", &[]);
        d.blocked_by = vec!["the owner".into()];
        let mut r = card("rr", "review", &[]);
        r.hours = 9.into();
        let b = board(vec![a, card("bb", "ready", &["aa"]), c, d, r]);
        let ids = |v: Vec<&Card>| v.iter().map(|c| c.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(next(&b, 3)), vec!["bb"], "deps done and nothing blocks");

        let p = plan(&b, "2026-11-16");
        assert_eq!((p.open_cards, p.open_hours), (3, 8.0), "the review card is built, not to build");
        assert_eq!((p.review_cards, p.review_hours), (1, 9.0));
        assert_eq!(p.chain, vec!["bb", "cc"], "the done card is not on the open chain");
        assert_eq!(p.chain_hours, 6.0);
        assert_eq!(p.late, vec!["bb", "cc", "dd"], "a review card waits for the owner, it is not late");
        assert_eq!(p.days, vec![
            Day { date: "2026-11-14".into(), hours: 4.0, over: false },
            Day { date: "2026-11-15".into(), hours: 4.0, over: false },
        ]);
        assert_eq!(p.day_hours, DAY_HOURS, "no day_hours on the board: one person's day");

        let mut tight = b.clone();
        tight.day_hours = Some(3.0);
        let p = plan(&tight, "2026-11-16");
        assert_eq!(p.day_hours, 3.0);
        assert!(p.days.iter().all(|d| d.over), "4 h on a 3 h day is over: {:?}", p.days);
        assert!(plan_text(&p).contains("a day holds 3 h") && plan_text(&p).contains("over 3 h"));

        assert_eq!(ideas(&[1, 2, 3, 4, 5, 8, 9, 10]), "1 to 5, 8 to 10");
        assert_eq!(ideas(&[1, 2, 7]), "1, 2, 7");
        assert_eq!(fmt_h(2.5), "2.5");
        assert_eq!(fmt_h(6.0), "6");
    }

    #[test]
    fn render_replaces_only_its_region() {
        let b = board(vec![card("aa", "ready", &[])]);
        let old = format!("# Board\n\nby hand\n\n{START}\nstale\n{END}\n\n## Kept\n");
        let new = render_readme(&old, &b).expect("render");
        assert!(new.starts_with("# Board\n\nby hand\n\n"));
        assert!(new.ends_with(&format!("{END}\n\n## Kept\n")));
        assert!(new.contains("| aa |  | S | 2 | 11-14 to 11-15 | | |"), "{new}");
        assert!(new.contains("Empty: backlog, doing, review, done, dropped."));
        assert_eq!(render_readme(&new, &b).expect("again"), new, "rendering twice changes nothing");
        assert!(render_readme("no markers", &b).is_err());
    }

    /// A board with a card in every list, some late and one blocked, for the views.
    fn spread() -> Board {
        let mut cards = Vec::new();
        for (i, list) in LISTS.iter().enumerate() {
            let mut c = card(&format!("card-{i}"), list, &[]);
            c.title = format!("A card in {list} with a title long enough to be cut by a narrow screen");
            c.x.insert("section".into(), serde_json::json!("board"));
            cards.push(c);
        }
        cards[1].blocked_by = vec!["the owner answers".into()];
        board(cards)
    }

    #[test]
    fn board_looks_plain_in_pipes_and_coloured_in_a_terminal() {
        let b = spread();
        let plain = board_view(&b, "2026-11-20", &Look::plain());
        assert!(!plain.contains('\x1b'), "no escapes in a pipe");
        assert!(plain.starts_with(&format!("{} (updated {})\n", b.board, b.updated)));
        assert!(plain.contains("[late, blocked]"), "{plain}");
        let paint = Paint::from_parts(true, false, None, Some("xterm-256color"));
        let look = Look::new(paint, 80);
        let tty = board_view(&b, "2026-11-20", &look);
        assert!(tty.contains("\x1b[1m"));
        for l in tty.lines().filter(|l| l.starts_with("  ")) {
            let bare = console::strip_ansi_codes(l);
            assert!(bare.chars().count() <= 80, "cut to the width: {bare}");
        }
        assert_eq!(fmt_h(hours(&[])), "0", "an empty sum is -0.0; it must print 0");
    }

    #[test]
    fn no_color_and_json_keep_the_board_plain() {
        let b = spread();
        let off = Paint::from_parts(true, false, Some("1"), Some("xterm"));
        assert!(!board_view(&b, "2026-11-20", &Look::new(off, 80)).contains('\x1b'));
        let json = Paint::from_parts(true, true, None, Some("xterm"));
        assert!(!board_view(&b, "2026-11-20", &Look::new(json, 80)).contains('\x1b'));
    }

    #[test]
    fn x_section_and_ideas_show_and_filter() {
        let mut a = card("aa", "ready", &[]);
        a.x.insert("section".into(), serde_json::json!("fit"));
        a.x.insert("ideas".into(), serde_json::json!([1, 2, 3, 7]));
        a.x.insert("track".into(), serde_json::json!("blue"));
        let b = board(vec![a, card("bb", "ready", &[])]);
        let only = Filter { section: Some("fit".into()), ..Filter::default() };
        assert_eq!(list(&b, &only, "2026-11-14").len(), 1);
        let text = show(&b.cards[0]);
        assert!(text.contains("section fit") && text.contains("ideas: 1 to 3, 7"), "{text}");
        assert!(text.contains("x.track: \"blue\""), "{text}");
    }
}
