//! `iohr rfc`: RFCs in the RFCs product (platform RFC 0065), through the Rust SDK's
//! `rfcs` operations. An RFC is named by its number (`0065`, a part `0065.1`) and found
//! in the account's spaces; `--space` narrows the search when two spaces share a number.

// Request bodies are built from `Default` and then set field by field, never as struct
// literals: the generated models become `#[non_exhaustive]` (sdk board, 0.3.0), where a
// literal from outside the crate no longer compiles.
#![allow(
    clippy::field_reassign_with_default,
    reason = "the generated models are about to be non_exhaustive"
)]

use std::io::Read as _;
use std::path::Path;

use inorbithr::public::{
    AddCommentRequest, CreateDiagramRequest, CreateDocumentRequest, Diagram, Document, Finding,
    RequestReviewRequest, RfcsGetDocumentParams, RfcsListDiagramsParams, RfcsListDocumentsParams,
    RfcsListSpacesParams, SaveDiagramRequest, SaveDocumentRequest, SetStatusRequest, Space,
    Surface as _,
};
use serde_json::{Value, json};

use crate::Env;
use crate::cli::{Global, RfcCommand, RfcWhere};
use crate::context::{Api, session};
use crate::error::Error;
use crate::output::Out;

/// The largest document the API takes (`SaveDocument`: 512 KiB).
const MAX_TEXT: usize = 512 * 1024;
/// The largest diagram model the API takes (1 MiB).
const MAX_MODEL: usize = 1024 * 1024;
/// The largest comment the API takes (10 KiB).
const MAX_COMMENT: usize = 10 * 1024;
/// The organisation a bare `repo#n` belongs to.
const DEFAULT_OWNER: &str = "inorbithr";

#[allow(clippy::too_many_lines, reason = "one arm per subcommand, each a call")]
pub(crate) async fn run(g: &Global, env: &Env, cmd: RfcCommand, out: Out) -> Result<(), Error> {
    let s = session(g, env).await?;
    let api = &s.api;
    match cmd {
        RfcCommand::List {
            at,
            status,
            query,
            mine,
        } => list(api, &at, status, query, mine, out).await,
        RfcCommand::Show {
            rfc,
            at,
            text,
            raw,
            at_version,
        } => show(api, &Ref::parse(&rfc)?, &at, text, raw, at_version, out).await,
        RfcCommand::Create {
            space,
            account,
            title,
            summary,
            parent,
            file,
            message,
        } => {
            let at = RfcWhere {
                space: Some(space),
                account,
            };
            let parent = parent.as_deref().map(Ref::parse).transpose()?;
            let text = file.as_deref().map(read_text).transpose()?;
            create(
                api,
                &at,
                &title,
                summary,
                parent.as_ref(),
                text,
                message,
                out,
            )
            .await
        }
        RfcCommand::Save {
            rfc,
            at,
            file,
            message,
            base,
        } => {
            let text = read_text(&file)?;
            save(api, &Ref::parse(&rfc)?, &at, text, &message, base, out).await
        }
        RfcCommand::Status {
            rfc,
            status,
            at,
            successor,
        } => {
            let successor = successor.as_deref().map(Ref::parse).transpose()?;
            set_status(
                api,
                &Ref::parse(&rfc)?,
                &at,
                &status,
                successor.as_ref(),
                out,
            )
            .await
        }
        RfcCommand::Comment {
            rfc,
            text,
            at,
            pr,
            anchor,
        } => {
            let line = match pr.pr.as_deref() {
                Some(raw) => Some(pr_line(
                    &pull_request(raw)?,
                    pr.title.as_deref(),
                    state(pr.merged, pr.live),
                )),
                None => None,
            };
            let body = comment_body(&text, line.as_deref());
            comment(api, &Ref::parse(&rfc)?, &at, body, anchor, out).await
        }
        RfcCommand::LinkPr {
            rfc,
            pr,
            at,
            title,
            merged,
            live,
        } => {
            let body = pr_line(&pull_request(&pr)?, title.as_deref(), state(merged, live));
            comment(api, &Ref::parse(&rfc)?, &at, body, None, out).await
        }
        RfcCommand::Diagram {
            id,
            at,
            file,
            name,
            message,
        } => {
            let model = read_model(&file)?;
            diagram(api, id.as_deref(), &at, model, name, message, out).await
        }
        RfcCommand::Review {
            rfc,
            at,
            reviewers,
            reason,
        } => review(api, &Ref::parse(&rfc)?, &at, reviewers, reason, out).await,
    }
}

/// How an RFC is named on the command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Ref {
    /// `0065` (part 0) or `0065.1` (part 1).
    Number { number: u32, part: u32 },
    /// `ldoc_...`.
    Id(String),
}

impl Ref {
    /// `0065`, `65`, `0065.1`, `RFC 0065` or a document id.
    pub(crate) fn parse(raw: &str) -> Result<Self, Error> {
        let t = raw.trim();
        let t = t
            .strip_prefix("RFC")
            .or_else(|| t.strip_prefix("rfc"))
            .map_or(t, str::trim_start);
        if let Some(id) = t.strip_prefix("ldoc_") {
            if !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric()) {
                return Ok(Self::Id(t.to_owned()));
            }
        } else {
            let (n, p) = t.split_once('.').unwrap_or((t, "0"));
            let digits = |s: &str, max: usize| {
                (1..=max).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit())
            };
            if digits(n, 4) && digits(p, 2) {
                let number: u32 = n.parse().unwrap_or(0);
                let part: u32 = p.parse().unwrap_or(0);
                if number > 0 && part <= 99 {
                    return Ok(Self::Number { number, part });
                }
            }
        }
        Err(Error::Usage(format!(
            "`{raw}` is not an RFC: give its number (0065, a part 0065.1) or its id (ldoc_...)"
        )))
    }

    fn matches(&self, d: &Document) -> bool {
        match self {
            Self::Id(id) => d.document_id == *id,
            Self::Number { number, part } => {
                d.kind == "rfc"
                    && u32::try_from(d.number).ok() == Some(*number)
                    && u32::try_from(d.child_index).ok() == Some(*part)
            }
        }
    }
}

impl std::fmt::Display for Ref {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Id(id) => f.write_str(id),
            Self::Number { number, part: 0 } => write!(f, "{number:04}"),
            Self::Number { number, part } => write!(f, "{number:04}.{part}"),
        }
    }
}

/// A pull request as `owner/repo#n`: from `repo#n` (an inorbithr repository),
/// `owner/repo#n` or `https://github.com/owner/repo/pull/n`.
pub(crate) fn pull_request(raw: &str) -> Result<String, Error> {
    let t = raw.trim();
    let bad = || {
        Error::Usage(format!(
            "`{raw}` is not a pull request: give repo#n, owner/repo#n or its GitHub link"
        ))
    };
    let (repo, n) = if let Some(rest) = t
        .strip_prefix("https://github.com/")
        .or_else(|| t.strip_prefix("github.com/"))
    {
        let parts: Vec<&str> = rest.trim_end_matches('/').split('/').collect();
        match parts.as_slice() {
            [owner, repo, "pull", n, ..] => (format!("{owner}/{repo}"), (*n).to_owned()),
            _ => return Err(bad()),
        }
    } else {
        let (repo, n) = t.split_once('#').ok_or_else(bad)?;
        let repo = if repo.contains('/') {
            repo.to_owned()
        } else {
            format!("{DEFAULT_OWNER}/{repo}")
        };
        (repo, n.to_owned())
    };
    let name_ok = |s: &str| {
        !s.is_empty()
            && s.len() <= 100
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    let repo_ok = repo
        .split_once('/')
        .is_some_and(|(o, r)| name_ok(o) && name_ok(r));
    let n_ok = (1..=9).contains(&n.len())
        && n.bytes().all(|b| b.is_ascii_digit())
        && n.parse::<u64>().is_ok_and(|v| v > 0);
    if repo_ok && n_ok {
        Ok(format!("{repo}#{n}"))
    } else {
        Err(bad())
    }
}

/// Where a pull request stands, in the words of its comment line.
pub(crate) fn state(merged: bool, live: bool) -> &'static str {
    if live {
        "live"
    } else if merged {
        "merged"
    } else {
        "open"
    }
}

/// A pull request's comment line, the same `mise run rfcs:comment` writes:
/// `owner/repo#n "title": open`, or `owner/repo#n: merged` without a title.
pub(crate) fn pr_line(pr: &str, title: Option<&str>, state: &str) -> String {
    let title = title
        .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|t| !t.is_empty());
    match title {
        Some(t) => format!("{pr} \"{t}\": {state}"),
        None => format!("{pr}: {state}"),
    }
}

/// A comment's text: the pull request's line first when there is one, then `. text`.
pub(crate) fn comment_body(text: &str, line: Option<&str>) -> String {
    let text = text.trim();
    match line {
        Some(l) if text.is_empty() => l.to_owned(),
        Some(l) => format!("{l}. {text}"),
        None => text.to_owned(),
    }
}

/// A document's text from a file, or from stdin for `-`.
fn read_text(path: &Path) -> Result<String, Error> {
    let text = if path == Path::new("-") {
        let mut s = String::new();
        std::io::stdin()
            .take(u64::try_from(MAX_TEXT).unwrap_or(u64::MAX) + 1)
            .read_to_string(&mut s)
            .map_err(|e| Error::Failed(format!("reading stdin: {e}")))?;
        s
    } else {
        std::fs::read_to_string(path)
            .map_err(|e| Error::Usage(format!("{}: {e}", path.display())))?
    };
    if text.len() > MAX_TEXT {
        return Err(Error::Usage(format!(
            "{}: a document is at most 512 KiB",
            path.display()
        )));
    }
    if text.trim().is_empty() {
        return Err(Error::Usage(format!(
            "{}: the file is empty",
            path.display()
        )));
    }
    Ok(text)
}

/// A diagram model from a file (or stdin for `-`): JSON with `nodes` and `edges`.
fn read_model(path: &Path) -> Result<String, Error> {
    let text = if path == Path::new("-") {
        let mut s = String::new();
        std::io::stdin()
            .take(u64::try_from(MAX_MODEL).unwrap_or(u64::MAX) + 1)
            .read_to_string(&mut s)
            .map_err(|e| Error::Failed(format!("reading stdin: {e}")))?;
        s
    } else {
        std::fs::read_to_string(path)
            .map_err(|e| Error::Usage(format!("{}: {e}", path.display())))?
    };
    if text.len() > MAX_MODEL {
        return Err(Error::Usage(format!(
            "{}: a diagram is at most 1 MiB",
            path.display()
        )));
    }
    let v: Value = serde_json::from_str(&text)
        .map_err(|e| Error::Usage(format!("{}: not JSON: {e}", path.display())))?;
    if !v.get("nodes").is_some_and(Value::is_array) || !v.get("edges").is_some_and(Value::is_array)
    {
        return Err(Error::Usage(format!(
            "{}: a diagram model has `nodes` and `edges` arrays",
            path.display()
        )));
    }
    Ok(text)
}

/// The spaces a command looks in: `--space` (slug or id) or every space of the account.
async fn spaces(api: &Api, at: &RfcWhere) -> Result<Vec<Space>, Error> {
    let mut params = RfcsListSpacesParams::default();
    params.account_id.clone_from(&at.account);
    let all = api
        .client()
        .rfcs()
        .all_list_spaces(&params)
        .collect()
        .await
        .map_err(scoped)?;
    let Some(want) = at.space.as_deref().map(str::trim) else {
        return Ok(all);
    };
    let found: Vec<Space> = all
        .into_iter()
        .filter(|s| s.slug == want || s.space_id == want)
        .collect();
    if found.is_empty() {
        return Err(Error::Usage(format!(
            "no space `{want}` in this account: `iohr rfc list` shows the spaces there are"
        )));
    }
    Ok(found)
}

async fn documents(
    api: &Api,
    space: &Space,
    params: &RfcsListDocumentsParams,
) -> Result<Vec<Document>, Error> {
    api.client()
        .rfcs()
        .all_list_documents(&space.space_id, params)
        .collect()
        .await
        .map_err(scoped)
}

/// The one document `r` names in the spaces `at` allows.
async fn find(api: &Api, r: &Ref, at: &RfcWhere) -> Result<(Space, Document), Error> {
    let mut params = RfcsListDocumentsParams::default();
    params.types = Some("rfc".to_owned());
    let mut hits: Vec<(Space, Document)> = Vec::new();
    for space in spaces(api, at).await? {
        for d in documents(api, &space, &params).await? {
            if r.matches(&d) {
                hits.push((space.clone(), d));
            }
        }
    }
    match hits.len() {
        0 => Err(Error::Usage(format!(
            "no RFC {r} that this credential can read{}",
            at.space
                .as_deref()
                .map_or_else(String::new, |s| format!(" in space {s}"))
        ))),
        1 => Ok(hits.remove(0)),
        _ => Err(Error::Usage(format!(
            "RFC {r} is in several spaces ({}): pass --space",
            hits.iter()
                .map(|(s, _)| s.slug.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

/// A 403 or 404 from the RFCs API, said in the words of what to do.
fn scoped(e: inorbithr::Error) -> Error {
    match e.status() {
        Some(403) => Error::with_hint(
            e,
            "Reading RFCs needs the rfc:read scope and writing rfc:write, and the account's \
             member who may write; settings and access need its owner or an admin.",
        ),
        Some(409) => Error::with_hint(
            e,
            "Someone saved a newer version: `iohr rfc show <rfc> --text --raw` gives it; \
             merge your change into it and save again.",
        ),
        _ => e.into(),
    }
}

fn number_of(d: &Document) -> String {
    if d.display_number.is_empty() {
        format!("{:04}", d.number)
    } else {
        d.display_number.clone()
    }
}

/// A document as `--json` prints it: the API's fields plus the space's slug.
fn with_space(d: &Document, space: &Space) -> Value {
    let mut v = serde_json::to_value(d).unwrap_or(Value::Null);
    if let Value::Object(m) = &mut v {
        m.insert("space".to_owned(), Value::String(space.slug.clone()));
    }
    v
}

fn note_findings(findings: &[Finding]) {
    for f in findings {
        Out::note(&format!(
            "finding {} (line {}): {}",
            f.rule, f.line, f.message
        ));
    }
}

async fn list(
    api: &Api,
    at: &RfcWhere,
    status: Option<String>,
    query: Option<String>,
    mine: bool,
    out: Out,
) -> Result<(), Error> {
    let mut params = RfcsListDocumentsParams::default();
    params.types = Some("rfc".to_owned());
    params.status = status;
    params.query = query;
    params.mine = mine.then_some(true);
    let mut rows: Vec<(Space, Document)> = Vec::new();
    for space in spaces(api, at).await? {
        for d in documents(api, &space, &params).await? {
            rows.push((space.clone(), d));
        }
    }
    rows.sort_by(|(a, x), (b, y)| {
        (x.number, x.child_index, &a.slug).cmp(&(y.number, y.child_index, &b.slug))
    });
    if out.json {
        let v: Vec<Value> = rows.iter().map(|(s, d)| with_space(d, s)).collect();
        Out::print_json(&v);
        return Ok(());
    }
    let table: Vec<Vec<String>> = rows
        .iter()
        .map(|(s, d)| {
            vec![
                number_of(d),
                s.slug.clone(),
                d.status.clone(),
                d.access.clone(),
                d.current_version.to_string(),
                d.title.clone(),
            ]
        })
        .collect();
    Out::table(
        &["RFC", "SPACE", "STATUS", "ACCESS", "VERSION", "TITLE"],
        &table,
    );
    Ok(())
}

async fn show(
    api: &Api,
    r: &Ref,
    at: &RfcWhere,
    text: bool,
    raw: bool,
    version: Option<u32>,
    out: Out,
) -> Result<(), Error> {
    let (space, d) = find(api, r, at).await?;
    let mut params = RfcsGetDocumentParams::default();
    params.version = version.map(|v| i32::try_from(v).unwrap_or(i32::MAX));
    let got = api
        .client()
        .rfcs()
        .get_document(&space.space_id, &d.document_id, &params)
        .await
        .map_err(scoped)?
        .value;
    if out.json {
        let mut v = serde_json::to_value(&got).unwrap_or(Value::Null);
        if let Value::Object(m) = &mut v {
            m.insert("space".to_owned(), Value::String(space.slug.clone()));
        }
        Out::print_json(&v);
        return Ok(());
    }
    if raw {
        Out::raw(got.text.as_bytes());
        return Ok(());
    }
    let doc = got.document.as_ref().unwrap_or(&d);
    Out::pairs(&[
        ("rfc", number_of(doc)),
        ("title", doc.title.clone()),
        ("space", space.slug.clone()),
        ("status", doc.status.clone()),
        ("access", doc.access.clone()),
        ("version", got.version.to_string()),
        ("saved", got.saved_at.clone()),
        ("by", got.author.clone()),
        ("id", doc.document_id.clone()),
        ("path", doc.path.clone()),
    ]);
    note_findings(&got.findings);
    if text {
        Out::raw(b"\n");
        Out::raw(got.text.as_bytes());
    }
    Ok(())
}

#[allow(
    clippy::too_many_arguments,
    reason = "one argument per flag of the command"
)]
async fn create(
    api: &Api,
    at: &RfcWhere,
    title: &str,
    summary: Option<String>,
    parent: Option<&Ref>,
    text: Option<String>,
    message: Option<String>,
    out: Out,
) -> Result<(), Error> {
    let found = spaces(api, at).await?;
    let [space] = found.as_slice() else {
        return Err(Error::Usage(
            "--space names more than one space; give its id".into(),
        ));
    };
    let parent_id = match parent {
        Some(p) => {
            let one = RfcWhere {
                space: Some(space.space_id.clone()),
                account: at.account.clone(),
            };
            Some(find(api, p, &one).await?.1.document_id)
        }
        None => None,
    };
    let mut body = CreateDocumentRequest::default();
    body.space_id = Some(space.space_id.clone());
    body.kind = Some("rfc".to_owned());
    body.title = Some(title.to_owned());
    body.summary = summary;
    body.parent_id = parent_id;
    let made = api
        .client()
        .rfcs()
        .create_document(&space.space_id, &body)
        .await
        .map_err(scoped)?
        .value;
    let doc = made
        .document
        .ok_or_else(|| Error::Failed("the API made the RFC but did not return it".into()))?;
    let mut findings = made.findings;
    let mut current = doc.clone();
    if let Some(text) = text {
        let mut save = SaveDocumentRequest::default();
        save.space_id = Some(space.space_id.clone());
        save.document_id = Some(doc.document_id.clone());
        save.base_version = Some(doc.current_version);
        save.text = Some(text);
        save.message = Some(message.unwrap_or_else(|| "First draft".to_owned()));
        let saved = api
            .client()
            .rfcs()
            .save_document(&space.space_id, &doc.document_id, &save)
            .await
            .map_err(scoped)?
            .value;
        findings = saved.findings;
        if let Some(d) = saved.document {
            current = d;
        }
    }
    if out.json {
        Out::print_json(&json!({
            "document": with_space(&current, space),
            "findings": findings,
        }));
        return Ok(());
    }
    Out::pairs(&[
        ("rfc", number_of(&current)),
        ("title", current.title.clone()),
        ("space", space.slug.clone()),
        ("version", current.current_version.to_string()),
        ("access", current.access.clone()),
        ("id", current.document_id.clone()),
    ]);
    note_findings(&findings);
    Ok(())
}

async fn save(
    api: &Api,
    r: &Ref,
    at: &RfcWhere,
    text: String,
    message: &str,
    base: Option<u32>,
    out: Out,
) -> Result<(), Error> {
    let (space, d) = find(api, r, at).await?;
    let mut body = SaveDocumentRequest::default();
    body.space_id = Some(space.space_id.clone());
    body.document_id = Some(d.document_id.clone());
    body.base_version =
        Some(base.map_or(d.current_version, |b| i32::try_from(b).unwrap_or(i32::MAX)));
    body.text = Some(text);
    body.message = Some(message.to_owned());
    let saved = api
        .client()
        .rfcs()
        .save_document(&space.space_id, &d.document_id, &body)
        .await
        .map_err(scoped)?
        .value;
    if out.json {
        Out::print_json(&saved);
        return Ok(());
    }
    let doc = saved.document.as_ref().unwrap_or(&d);
    Out::pairs(&[
        ("rfc", number_of(doc)),
        ("version", saved.version.to_string()),
        ("status", doc.status.clone()),
    ]);
    if saved.version == d.current_version {
        Out::note("The text is the same as the current version: nothing was saved.");
    }
    note_findings(&saved.findings);
    Ok(())
}

async fn set_status(
    api: &Api,
    r: &Ref,
    at: &RfcWhere,
    status: &str,
    successor: Option<&Ref>,
    out: Out,
) -> Result<(), Error> {
    let (space, d) = find(api, r, at).await?;
    let successor_id = match successor {
        Some(s) => {
            let one = RfcWhere {
                space: Some(space.space_id.clone()),
                account: at.account.clone(),
            };
            Some(find(api, s, &one).await?.1.document_id)
        }
        None => None,
    };
    let mut body = SetStatusRequest::default();
    body.space_id = Some(space.space_id.clone());
    body.document_id = Some(d.document_id.clone());
    body.status = Some(status.trim().to_owned());
    body.successor_id = successor_id;
    body.base_version = Some(d.current_version);
    let set = api
        .client()
        .rfcs()
        .set_status(&space.space_id, &d.document_id, &body)
        .await
        .map_err(scoped)?
        .value;
    if out.json {
        Out::print_json(&set);
        return Ok(());
    }
    let doc = set.document.as_ref().unwrap_or(&d);
    Out::pairs(&[
        ("rfc", number_of(doc)),
        ("status", doc.status.clone()),
        ("version", doc.current_version.to_string()),
    ]);
    Ok(())
}

async fn comment(
    api: &Api,
    r: &Ref,
    at: &RfcWhere,
    body: String,
    anchor: Option<String>,
    out: Out,
) -> Result<(), Error> {
    if body.trim().is_empty() || body.len() > MAX_COMMENT {
        return Err(Error::Usage("a comment is 1 byte to 10 KiB".into()));
    }
    let (space, d) = find(api, r, at).await?;
    let made = add_comment(api, &space, &d, body, anchor).await?;
    if out.json {
        Out::print_json(&made);
        return Ok(());
    }
    let id = made
        .get("comment")
        .map_or("", |c| super::str_of(c, "comment_id"));
    Out::pairs(&[("rfc", number_of(&d)), ("comment", id.to_owned())]);
    Ok(())
}

async fn add_comment(
    api: &Api,
    space: &Space,
    d: &Document,
    body: String,
    anchor: Option<String>,
) -> Result<Value, Error> {
    let mut req = AddCommentRequest::default();
    req.space_id = Some(space.space_id.clone());
    req.document_id = Some(d.document_id.clone());
    req.body = Some(body);
    req.anchor = anchor;
    let made = api
        .client()
        .rfcs()
        .add_comment(&space.space_id, &d.document_id, &req)
        .await
        .map_err(scoped)?
        .value;
    Ok(serde_json::to_value(&made).unwrap_or(Value::Null))
}

#[allow(
    clippy::too_many_arguments,
    reason = "one argument per flag of the command"
)]
async fn diagram(
    api: &Api,
    id: Option<&str>,
    at: &RfcWhere,
    model: String,
    name: Option<String>,
    message: Option<String>,
    out: Out,
) -> Result<(), Error> {
    let found = spaces(api, at).await?;
    let (space, made): (Space, Diagram) = if let Some(id) = id {
        let id = id.trim();
        if !id.starts_with("ldia_") {
            return Err(Error::Usage(format!(
                "`{id}` is not a diagram id (ldia_...)"
            )));
        }
        let mut hit: Option<(Space, Diagram)> = None;
        for space in found {
            let all = api
                .client()
                .rfcs()
                .all_list_diagrams(&space.space_id, &RfcsListDiagramsParams::default())
                .collect()
                .await
                .map_err(scoped)?;
            if let Some(d) = all.into_iter().find(|d| d.diagram_id == id) {
                hit = Some((space, d));
                break;
            }
        }
        let (space, current) = hit.ok_or_else(|| {
            Error::Usage(format!("no diagram {id} that this credential can read"))
        })?;
        let mut body = SaveDiagramRequest::default();
        body.space_id = Some(space.space_id.clone());
        body.diagram_id = Some(current.diagram_id.clone());
        body.base_version = Some(current.current_version);
        body.model = Some(model);
        body.message = message;
        body.name = name;
        let saved = api
            .client()
            .rfcs()
            .save_diagram(&space.space_id, &current.diagram_id, &body)
            .await
            .map_err(scoped)?
            .value;
        let d = saved.diagram.unwrap_or(current);
        (space, d)
    } else {
        let [space] = found.as_slice() else {
            return Err(Error::Usage(
                "a new diagram goes in one space: pass --space".into(),
            ));
        };
        let name = name.ok_or_else(|| Error::Usage("a new diagram needs --name".into()))?;
        let mut body = CreateDiagramRequest::default();
        body.space_id = Some(space.space_id.clone());
        body.name = Some(name);
        body.model = Some(model);
        let created = api
            .client()
            .rfcs()
            .create_diagram(&space.space_id, &body)
            .await
            .map_err(scoped)?
            .value;
        let d = created.diagram.ok_or_else(|| {
            Error::Failed("the API made the diagram but did not return it".into())
        })?;
        (space.clone(), d)
    };
    if out.json {
        let mut v = serde_json::to_value(&made).unwrap_or(Value::Null);
        if let Value::Object(m) = &mut v {
            m.insert("space".to_owned(), Value::String(space.slug.clone()));
        }
        Out::print_json(&v);
        return Ok(());
    }
    Out::pairs(&[
        ("diagram", made.diagram_id.clone()),
        ("name", made.name.clone()),
        ("space", space.slug.clone()),
        ("version", made.current_version.to_string()),
        ("nodes", made.nodes.to_string()),
        ("embed", format!("```diagram {}", made.diagram_id)),
    ]);
    Ok(())
}

async fn review(
    api: &Api,
    r: &Ref,
    at: &RfcWhere,
    reviewers: Vec<String>,
    reason: Option<String>,
    out: Out,
) -> Result<(), Error> {
    let reviewers: Vec<String> = reviewers
        .into_iter()
        .map(|r| r.trim().to_owned())
        .filter(|r| !r.is_empty())
        .collect();
    if reviewers.is_empty() {
        return Err(Error::Usage(
            "name a reviewer with --reviewer or IOHR_RFC_REVIEWER".into(),
        ));
    }
    if let Some(reason) = reason.as_deref()
        && (reason.trim().is_empty() || reason.len() > MAX_COMMENT - 64)
    {
        return Err(Error::Usage("--reason is 1 byte to about 10 KiB".into()));
    }
    let (space, d) = find(api, r, at).await?;
    let mut req = RequestReviewRequest::default();
    req.space_id = Some(space.space_id.clone());
    req.document_id = Some(d.document_id.clone());
    req.reviewers = reviewers;
    let asked = api
        .client()
        .rfcs()
        .request_review(&space.space_id, &d.document_id, &req)
        .await
        .map_err(scoped)?
        .value;
    let comment = match reason {
        Some(reason) => Some(
            add_comment(
                api,
                &space,
                &d,
                format!(
                    "Review requested: {}",
                    reason.split_whitespace().collect::<Vec<_>>().join(" ")
                ),
                None,
            )
            .await?,
        ),
        None => None,
    };
    if out.json {
        Out::print_json(&json!({ "reviews": asked.reviews, "comment": comment }));
        return Ok(());
    }
    let rows: Vec<Vec<String>> = asked
        .reviews
        .iter()
        .map(|v| {
            vec![
                number_of(&d),
                v.reviewer.clone(),
                v.version.to_string(),
                v.review_id.clone(),
            ]
        })
        .collect();
    Out::table(&["RFC", "REVIEWER", "VERSION", "REVIEW"], &rows);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc_numbers_and_ids_parse() {
        assert_eq!(
            Ref::parse("0065").unwrap(),
            Ref::Number {
                number: 65,
                part: 0
            }
        );
        assert_eq!(
            Ref::parse("RFC 0040.14").unwrap(),
            Ref::Number {
                number: 40,
                part: 14
            }
        );
        assert_eq!(
            Ref::parse("65").unwrap(),
            Ref::Number {
                number: 65,
                part: 0
            }
        );
        assert_eq!(
            Ref::parse("ldoc_01ABC").unwrap(),
            Ref::Id("ldoc_01ABC".into())
        );
        for bad in [
            "", "0", "0065.", "0065.100", "12345", "x65", "ldoc_", "ldoc_a/b",
        ] {
            assert!(Ref::parse(bad).is_err(), "{bad}");
        }
        assert_eq!(Ref::parse("0065.1").unwrap().to_string(), "0065.1");
        assert_eq!(Ref::parse("7").unwrap().to_string(), "0007");
    }

    #[test]
    fn pull_requests_read_three_ways() {
        assert_eq!(pull_request("core#512").unwrap(), "inorbithr/core#512");
        assert_eq!(
            pull_request("inorbithr/sdk#160").unwrap(),
            "inorbithr/sdk#160"
        );
        assert_eq!(
            pull_request("https://github.com/inorbithr/core/pull/499/files").unwrap(),
            "inorbithr/core#499"
        );
        for bad in [
            "core",
            "core#",
            "core#0",
            "core#x",
            "a b#1",
            "o/r/x#1",
            "https://github.com/o/r/issues/1",
        ] {
            assert!(pull_request(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn comment_lines_match_the_interim_task() {
        let pr = "inorbithr/core#509";
        assert_eq!(
            pr_line(pr, Some(" docs(rfcs):  status "), "open"),
            "inorbithr/core#509 \"docs(rfcs): status\": open"
        );
        assert_eq!(
            pr_line(pr, None, state(true, false)),
            "inorbithr/core#509: merged"
        );
        assert_eq!(
            pr_line(pr, Some(""), state(false, true)),
            "inorbithr/core#509: live"
        );
        assert_eq!(comment_body(" text ", None), "text");
        assert_eq!(
            comment_body("verified", Some("inorbithr/core#509: live")),
            "inorbithr/core#509: live. verified"
        );
        assert_eq!(comment_body("", Some("x#1: open")), "x#1: open");
    }
}
