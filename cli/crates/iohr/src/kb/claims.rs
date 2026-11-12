//! Crawl pages as evidence and extracted facts as claims (RFC 0218, slice 1).
//!
//! A page read by the crawl becomes an evidence record keyed by the digest of its bytes
//! ([`PageEvidence`]): the same bytes are one record however many addresses served them, and
//! reading unchanged bytes again is the same record. Each fact the site model extracted becomes
//! a claim of Atlas core ([`iohr_atlas_core`]) whose proof tree ends at the page it came from.
//! The ids of both come from their content, so [`ingest`] run again on the same input adds no
//! row (RFC 0218, idempotent ingest). A claim whose fact is gone, or whose page changed, is
//! marked stale, never deleted: stale is not false (ADR 0003).
//!
//! What "verified" means here is narrow and said so: a deterministic reader found this value
//! in these bytes (rule `existence@1`, ADR 0017). It does not say the value is true.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use chrono::{TimeDelta, Utc};
use duckdb::{Transaction, params};
use iohr_atlas_core::claim::{Claim, ClaimClass, Scope, Support, Validity};
use iohr_atlas_core::derivation::proof_tree;
use iohr_atlas_core::evidence::{EvidenceItem, EvidenceRef};
use iohr_atlas_core::ids::{ArtifactObservationId, ChangeId, ClaimId, EntityId, ObserverId};
use iohr_atlas_core::method::{CoverageModel, DataClass, EvidenceMethod, MethodCategory, Trust};
use iohr_atlas_core::observation::{ArtifactObservation, Polarity, Statement};
use iohr_atlas_core::observer::{Health, ObserverClass};
use iohr_atlas_core::proof::{EvidenceView, ExistenceRule, prove};
use iohr_atlas_core::snapshot::{ComponentKey, ComponentState, Revision, WorldSnapshot};
use iohr_atlas_core::time::{ClockQuality, ObservedTime, Timestamp};
use iohr_atlas_core::vocabulary::{PredicateName, PredicateRef, Value};

use super::index::Store;
use super::model::{Fact, Model};

/// The crawl's name as an observer: one stable identity, so evidence it records is dependent
/// on it (independent corroboration needs a second observer).
const OBSERVER: &[u8] = b"iohr.crawl";

fn err(what: &str, e: &duckdb::Error) -> String {
    format!("the local index: {what}: {e}")
}

fn bad(what: &str, e: impl std::fmt::Display) -> String {
    format!("the local index: {what}: {e}")
}

/// One page the crawl read: where it was, where the raw copy is kept, and the observation of
/// its bytes. The digest is computed from the bytes by Atlas core, never passed in.
#[derive(Debug, Clone)]
pub struct PageEvidence {
    pub url: String,
    /// Where the crawl kept the raw copy, relative to the crawl folder.
    pub cache: String,
    observation: ArtifactObservation,
    bytes: u64,
}

impl PageEvidence {
    /// The evidence of `bytes` read from `url` and kept at `cache`.
    ///
    /// # Errors
    ///
    /// A message when the observation cannot be made.
    pub fn read(url: &str, cache: &str, bytes: &[u8]) -> Result<Self, String> {
        let when = ObservedTime {
            wall: Utc::now(),
            boot_id: "unknown".to_owned(),
            monotonic_ns: None,
            clock: ClockQuality {
                source: "unknown".to_owned(),
                uncertainty: TimeDelta::seconds(1),
            },
        };
        let raw = ArtifactObservation::of_bytes(
            ObserverId::derived(&[OBSERVER]),
            ObserverClass::DeterministicExtractor,
            url.to_owned(),
            bytes,
            when,
        )
        .map_err(|e| bad("cannot observe a page", e))?;
        // The same bytes are the same observation, wherever they were read.
        let id = ArtifactObservationId::derived(&[raw.artifact().digest.bytes()]);
        Ok(Self {
            url: url.to_owned(),
            cache: cache.to_owned(),
            observation: raw.with_id(id),
            bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        })
    }

    /// The digest of the page's bytes, `sha256:` and hex.
    #[must_use]
    pub fn digest(&self) -> String {
        self.observation.artifact().digest.to_hex()
    }

    fn id(&self) -> ArtifactObservationId {
        self.observation.id()
    }
}

/// What one [`ingest`] changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Ingested {
    /// Evidence records new to the file.
    pub evidence_added: usize,
    /// Claims new to the file.
    pub claims_added: usize,
    /// Claims that went stale: their fact is gone or their page changed.
    pub claims_stale: usize,
    /// Facts with no page evidence (the page could not be read), so no claim.
    pub facts_unproven: usize,
}

/// How a page is told apart in a snapshot.
fn component(digest: &str) -> Result<ComponentKey, String> {
    let short: String = digest
        .trim_start_matches("sha256:")
        .chars()
        .take(16)
        .collect();
    ComponentKey::try_from(format!("page/{short}")).map_err(|e| bad("page component", e))
}

fn method() -> Result<EvidenceMethod, String> {
    Ok(EvidenceMethod {
        name: "crawl.page_read"
            .to_owned()
            .try_into()
            .map_err(|e| bad("method name", e))?,
        category: MethodCategory::ExternalRecord,
        semantics_version: 1,
        coverage: CoverageModel::BestEffort,
        // A page can be edited after the fact.
        trust: Trust::Mutable,
        classification: DataClass::Public,
        capabilities: BTreeSet::new(),
    })
}

/// The predicate name of a model fact: its own, under the `web` namespace.
fn predicate(fact: &Fact) -> Result<PredicateRef, String> {
    let clean: String = fact
        .predicate
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let name = PredicateName::try_from(format!("web.{clean}")).map_err(|e| bad("predicate", e))?;
    Ok(PredicateRef { name, version: 1 })
}

/// The claim of one fact on one page, proven by the deterministic read of those bytes.
fn claim_of(site: &str, fact: &Fact, page: &PageEvidence) -> Result<Claim, String> {
    let key = component(&page.digest())?;
    let snapshot = WorldSnapshot::new(BTreeMap::from([(
        key.clone(),
        ComponentState::known(Revision::Config(page.observation.artifact().digest)),
    )]))
    .map_err(|e| bad("snapshot", e))?;
    let scope = Scope {
        snapshot: snapshot.id(),
        components: BTreeSet::from([key]),
    };
    let statement = Statement {
        subject: EntityId::derived(&[site.as_bytes(), fact.subject.as_bytes()]),
        predicate: predicate(fact)?,
        value: Value::Text(fact.value.clone()),
        polarity: Polarity::Holds,
    };
    let id = ClaimId::derived(&[
        site.as_bytes(),
        fact.id.as_bytes(),
        page.observation.artifact().digest.bytes(),
    ]);
    let mut claim = Claim::new(statement, ClaimClass::Existence, scope).with_id(id);
    let item = EvidenceItem::new(
        EvidenceRef::Artifact(page.id()),
        ObserverClass::DeterministicExtractor,
        page.observation.observer(),
        method()?,
    )
    .map_err(|e| bad("evidence", e))?;
    let items = [item];
    let view = EvidenceView {
        items: &items,
        contrary: &[],
        coverage: None,
        health: Health::Healthy,
        authority: None,
        calibration: None,
    };
    prove(&ExistenceRule, &mut claim, &view);
    Ok(claim)
}

fn exists(tx: &Transaction<'_>, sql: &str, args: &[&str]) -> Result<bool, String> {
    let n: i64 = tx
        .query_row(sql, duckdb::params_from_iter(args.iter()), |r| r.get(0))
        .map_err(|e| err("cannot read", &e))?;
    Ok(n > 0)
}

/// The word for a support level, as the list and the proof show it.
#[must_use]
pub fn support_word(s: &Support) -> &'static str {
    match s {
        Support::None => "none",
        Support::Observed => "observed",
        Support::Supported => "supported",
        Support::Verified(_) => "verified",
    }
}

/// Records `pages` as evidence and `model`'s facts as claims of `site`, inside `tx`. Run again
/// on the same input it adds nothing. `crawled` names the crawl that made claims stale.
///
/// # Errors
///
/// A message when a row cannot be written; the caller rolls the transaction back.
pub(super) fn ingest(
    tx: &Transaction<'_>,
    site: &str,
    model: &Model,
    pages: &[PageEvidence],
    crawled: &str,
) -> Result<Ingested, String> {
    let mut done = Ingested::default();
    let now: Timestamp = Utc::now();
    let by_url: HashMap<&str, &PageEvidence> = pages.iter().map(|p| (p.url.as_str(), p)).collect();
    for p in pages {
        let id = p.id().to_string();
        if !exists(tx, "SELECT count(*) FROM evidence WHERE id = ?", &[&id])? {
            let json = serde_json::to_string(&p.observation).map_err(|e| bad("evidence", e))?;
            tx.execute(
                "INSERT INTO evidence VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    id,
                    p.digest(),
                    i64::try_from(p.bytes).unwrap_or(i64::MAX),
                    p.observation.observer().to_string(),
                    "deterministic_extractor",
                    "crawl.page_read@1",
                    now.to_rfc3339(),
                    json
                ],
            )
            .map_err(|e| err("cannot add evidence", &e))?;
            done.evidence_added += 1;
        }
        if !exists(
            tx,
            "SELECT count(*) FROM evidence_at WHERE evidence_id = ? AND site = ? AND url = ?",
            &[&id, site, &p.url],
        )? {
            tx.execute(
                "INSERT INTO evidence_at VALUES (?, ?, ?, ?)",
                params![id, site, p.url, p.cache],
            )
            .map_err(|e| err("cannot add where evidence was read", &e))?;
        }
    }
    let mut current: HashSet<String> = HashSet::new();
    for f in &model.facts {
        let Some(page) = by_url.get(f.url.as_str()) else {
            done.facts_unproven += 1;
            continue;
        };
        let claim = claim_of(site, f, page)?;
        let id = claim.id().to_string();
        current.insert(id.clone());
        if exists(tx, "SELECT count(*) FROM claims WHERE id = ?", &[&id])? {
            continue;
        }
        let json = serde_json::to_string(&claim).map_err(|e| bad("claim", e))?;
        let Value::Text(value) = &claim.statement().value else {
            continue;
        };
        tx.execute(
            "INSERT INTO claims VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                id,
                site,
                f.id,
                f.subject,
                claim.statement().predicate.name.as_str(),
                value,
                "existence",
                support_word(claim.state().support()),
                false,
                now.to_rfc3339(),
                json
            ],
        )
        .map_err(|e| err("cannot add a claim", &e))?;
        tx.execute(
            "INSERT INTO claim_evidence VALUES (?, ?)",
            params![id, page.id().to_string()],
        )
        .map_err(|e| err("cannot link a claim to its evidence", &e))?;
        done.claims_added += 1;
    }
    done.claims_stale = stale(tx, site, &current, crawled, now)?;
    Ok(done)
}

/// Marks the site's claims that are not in `current` stale.
fn stale(
    tx: &Transaction<'_>,
    site: &str,
    current: &HashSet<String>,
    crawled: &str,
    now: Timestamp,
) -> Result<usize, String> {
    let rows: Vec<(String, String)> = {
        let mut q = tx
            .prepare("SELECT id, json FROM claims WHERE site = ? AND NOT stale")
            .map_err(|e| err("cannot read claims", &e))?;
        q.query_map(params![site], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| err("cannot read claims", &e))?
            .collect::<Result<_, _>>()
            .map_err(|e| err("cannot read claims", &e))?
    };
    let change = ChangeId::derived(&[site.as_bytes(), crawled.as_bytes()]);
    let mut n = 0;
    for (id, json) in rows.into_iter().filter(|(id, _)| !current.contains(id)) {
        let mut claim: Claim = serde_json::from_str(&json).map_err(|e| bad("claim", e))?;
        claim.state_mut().mark_stale(change, now);
        let json = serde_json::to_string(&claim).map_err(|e| bad("claim", e))?;
        tx.execute(
            "UPDATE claims SET stale = true, json = ? WHERE id = ?",
            params![json, id],
        )
        .map_err(|e| err("cannot mark a claim stale", &e))?;
        n += 1;
    }
    Ok(n)
}

/// A claim as a list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimRow {
    pub id: String,
    pub site: String,
    pub subject: String,
    pub predicate: String,
    pub value: String,
    pub support: String,
    pub stale: bool,
}

/// One piece of evidence a proof ends at, with where it was read and where the raw copy is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceRow {
    pub id: String,
    pub digest: String,
    pub bytes: u64,
    /// `(url, cache path relative to the crawl folder)`.
    pub at: Vec<(String, String)>,
}

/// Why a claim is believed: its tree down to evidence, and the evidence it ends at.
#[derive(Debug, Clone)]
pub struct Proof {
    pub claim: Claim,
    pub tree: iohr_atlas_core::derivation::ProofTree,
    pub evidence: Vec<EvidenceRow>,
    /// The claim's certificate binds to exactly this claim (Atlas core's check).
    pub integrity: bool,
    pub stale: bool,
}

impl Store {
    /// The claims of `site` (or all), newest first.
    ///
    /// # Errors
    ///
    /// A message when the file cannot be read, or has no claim tables yet (a crawl brings it up).
    pub fn claims(&self, site: Option<&str>, limit: usize) -> Result<Vec<ClaimRow>, String> {
        let mut q = self
            .conn()
            .prepare(
                "SELECT id, site, subject, predicate, value, support, stale FROM claims \
                 WHERE (? IS NULL OR site = ?) ORDER BY recorded_at DESC, id LIMIT ?",
            )
            .map_err(|e| err("cannot read claims", &e))?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        q.query_map(params![site, site, limit], |r| {
            Ok(ClaimRow {
                id: r.get(0)?,
                site: r.get(1)?,
                subject: r.get(2)?,
                predicate: r.get(3)?,
                value: r.get(4)?,
                support: r.get(5)?,
                stale: r.get(6)?,
            })
        })
        .map_err(|e| err("cannot read claims", &e))?
        .collect::<Result<_, _>>()
        .map_err(|e| err("cannot read claims", &e))
    }

    /// The proof of the claim `id`: its tree to evidence, and each piece of evidence with the
    /// places it was read.
    ///
    /// # Errors
    ///
    /// A message when the claim is not there, or a link of its tree is missing.
    pub fn proof(&self, id: &str) -> Result<Proof, String> {
        let mut claims: BTreeMap<ClaimId, Claim> = BTreeMap::new();
        let mut want = vec![id.to_owned()];
        let mut first: Option<ClaimId> = None;
        while let Some(next) = want.pop() {
            let json: String = self
                .conn()
                .query_row("SELECT json FROM claims WHERE id = ?", params![next], |r| {
                    r.get(0)
                })
                .map_err(|_| format!("no claim {next} in the local index"))?;
            let claim: Claim = serde_json::from_str(&json).map_err(|e| bad("claim", e))?;
            first.get_or_insert(claim.id());
            if let Some(d) = claim.derivation() {
                want.extend(d.inputs().iter().map(ToString::to_string));
            }
            claims.insert(claim.id(), claim);
        }
        let root = first.ok_or_else(|| format!("no claim {id} in the local index"))?;
        let tree = proof_tree(&claims, root).map_err(|e| bad("proof tree", e))?;
        let claim = claims.remove(&root).ok_or("claim vanished")?;
        let mut evidence = Vec::new();
        for r in claim.evidence() {
            if let EvidenceRef::Artifact(a) = r {
                evidence.push(self.evidence_row(&a.to_string())?);
            }
        }
        let stale = matches!(claim.state().validity(), Validity::Stale { .. });
        Ok(Proof {
            integrity: claim.integrity_holds(),
            claim,
            tree,
            evidence,
            stale,
        })
    }

    fn evidence_row(&self, id: &str) -> Result<EvidenceRow, String> {
        let (digest, bytes): (String, i64) = self
            .conn()
            .query_row(
                "SELECT digest, bytes FROM evidence WHERE id = ?",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| format!("evidence {id} is missing from the local index"))?;
        let mut q = self
            .conn()
            .prepare("SELECT url, cache FROM evidence_at WHERE evidence_id = ? ORDER BY url")
            .map_err(|e| err("cannot read evidence", &e))?;
        let at = q
            .query_map(params![id], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| err("cannot read evidence", &e))?
            .collect::<Result<_, _>>()
            .map_err(|e| err("cannot read evidence", &e))?;
        Ok(EvidenceRow {
            id: id.to_owned(),
            digest,
            bytes: u64::try_from(bytes).unwrap_or(0),
            at,
        })
    }
}
