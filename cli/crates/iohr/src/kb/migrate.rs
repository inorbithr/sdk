//! The numbered migrations of the index file (RFC 0218). The file is the one
//! `.inorbit/crawl/kb.duckdb` the crawl already writes; a migration only adds to it. Each runs in
//! one transaction with its row in `schema_migrations`, so a file is at a version or the one
//! before, never between; a failed migration leaves the file as it was.
//!
//! Version 1 is the index as it was before migrations existed (`CREATE TABLE IF NOT EXISTS`, so
//! a file written by an older build is adopted as it is, its pages and chunks untouched).
//! Version 2 adds the claim and evidence tables.

use duckdb::{Connection, params};

/// The index tables of the first build: pages, chunks, terms and the site model.
pub(super) const V1: &str = "
CREATE TABLE IF NOT EXISTS pages(site TEXT, url TEXT, title TEXT, description TEXT, lang TEXT,
  depth INTEGER, chunks INTEGER, tokens INTEGER, links INTEGER);
CREATE TABLE IF NOT EXISTS chunks(id TEXT, site TEXT, url TEXT, ord INTEGER, heading TEXT,
  path TEXT, start_char INTEGER, end_char INTEGER, tokens INTEGER, est_model_tokens INTEGER,
  text TEXT);
CREATE TABLE IF NOT EXISTS terms(chunk_id TEXT, term TEXT, tf INTEGER);
CREATE TABLE IF NOT EXISTS entities(site TEXT, kind TEXT, name TEXT, mentions INTEGER,
  pages INTEGER, url TEXT, chunk TEXT, source TEXT);
CREATE TABLE IF NOT EXISTS facts(site TEXT, id TEXT, subject TEXT, predicate TEXT, value TEXT,
  url TEXT, chunk TEXT, source TEXT);
CREATE TABLE IF NOT EXISTS sections(site TEXT, name TEXT, pages INTEGER, chunks INTEGER,
  tokens INTEGER, summary TEXT);
CREATE TABLE IF NOT EXISTS links(site TEXT, from_url TEXT, to_url TEXT, text TEXT,
  internal BOOLEAN);
CREATE TABLE IF NOT EXISTS sites(site TEXT, crawled TEXT, built TEXT, embeddings TEXT,
  subject TEXT);
";

/// Claims and the evidence they end at. `evidence` is keyed by content hash: the same bytes are
/// one record however many addresses served them (`evidence_at` lists the addresses and where
/// the raw copy is kept). A claim's whole record (statement, proof, state) is `json`, written by
/// Atlas core; the other columns are what a query filters on.
const V2: &str = "
CREATE TABLE IF NOT EXISTS evidence(id TEXT, digest TEXT, bytes BIGINT, observer TEXT,
  class TEXT, method TEXT, observed_at TEXT, json TEXT);
CREATE TABLE IF NOT EXISTS evidence_at(evidence_id TEXT, site TEXT, url TEXT, cache TEXT);
CREATE TABLE IF NOT EXISTS claims(id TEXT, site TEXT, fact_id TEXT, subject TEXT,
  predicate TEXT, value TEXT, class TEXT, support TEXT, stale BOOLEAN, recorded_at TEXT,
  json TEXT);
CREATE TABLE IF NOT EXISTS claim_evidence(claim_id TEXT, evidence_id TEXT);
";

/// Every migration, oldest first: its version, a name and its SQL.
const ALL: &[(i64, &str, &str)] = &[(1, "pages_chunks_model", V1), (2, "claims_evidence", V2)];

/// The newest version this build knows.
pub const LATEST: i64 = 2;

fn err(what: &str, e: &duckdb::Error) -> String {
    format!("the local index: {what}: {e}")
}

/// The version the file is at: 0 when it has no migrations table (a new file, or one written
/// before migrations).
///
/// # Errors
///
/// A message when the file cannot be read.
pub fn version(conn: &Connection) -> Result<i64, String> {
    let has: i64 = conn
        .query_row(
            "SELECT count(*) FROM information_schema.tables WHERE table_name = 'schema_migrations'",
            [],
            |r| r.get(0),
        )
        .map_err(|e| err("cannot read its version", &e))?;
    if has == 0 {
        return Ok(0);
    }
    conn.query_row(
        "SELECT coalesce(max(version), 0) FROM schema_migrations",
        [],
        |r| r.get(0),
    )
    .map_err(|e| err("cannot read its version", &e))
}

/// Brings the file to [`LATEST`], returning the version it is at.
///
/// # Errors
///
/// A message when a migration fails (the file keeps its version) or when the file was written
/// by a newer build than this one, which must not be changed by an older one.
pub fn apply(conn: &Connection) -> Result<i64, String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations(version BIGINT, name TEXT, applied TEXT)",
    )
    .map_err(|e| err("cannot make its migrations table", &e))?;
    let at = version(conn)?;
    if at > LATEST {
        return Err(format!(
            "the local index is at version {at}, newer than this iohr knows ({LATEST}); update iohr"
        ));
    }
    for (v, name, sql) in ALL.iter().filter(|(v, _, _)| *v > at) {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| err("cannot start a migration", &e))?;
        tx.execute_batch(sql)
            .map_err(|e| err(&format!("migration {v} ({name}) failed"), &e))?;
        tx.execute(
            "INSERT INTO schema_migrations VALUES (?, ?, ?)",
            params![v, name, crate::front::today()],
        )
        .map_err(|e| err("cannot record a migration", &e))?;
        tx.commit()
            .map_err(|e| err(&format!("cannot save migration {v}"), &e))?;
    }
    Ok(LATEST)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_file_gets_every_migration_once_and_a_second_run_changes_nothing() {
        let c = Connection::open_in_memory().unwrap();
        assert_eq!(version(&c).unwrap(), 0);
        assert_eq!(apply(&c).unwrap(), LATEST);
        assert_eq!(apply(&c).unwrap(), LATEST);
        let n: i64 = c
            .query_row("SELECT count(*) FROM schema_migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, i64::try_from(ALL.len()).unwrap());
    }

    #[test]
    fn a_file_from_a_newer_build_is_not_touched() {
        let c = Connection::open_in_memory().unwrap();
        apply(&c).unwrap();
        c.execute(
            "INSERT INTO schema_migrations VALUES (99, 'later', 'x')",
            [],
        )
        .unwrap();
        let e = apply(&c).unwrap_err();
        assert!(e.contains("update iohr"), "{e}");
    }

    #[test]
    fn the_versions_are_dense_and_the_latest_is_the_last() {
        for (i, (v, _, _)) in ALL.iter().enumerate() {
            assert_eq!(*v, i64::try_from(i).unwrap() + 1);
        }
        assert_eq!(ALL.last().map(|m| m.0), Some(LATEST));
    }
}
