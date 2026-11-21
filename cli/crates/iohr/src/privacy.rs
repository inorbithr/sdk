//! The privacy filter InOrbit's service applies to a Role (ADR 0063, "classified stays with
//! owner"), so `iohr` can refuse a line here, with a clear message, before the service answers
//! a whole import with a 400. Memory uses its secret shapes too ([`has_secret`]).
//!
//! The service is the authority; this is its copy. The service's source is private, so this
//! public crate cannot build from it: the patterns below are copied whole from core
//! `crates/coordinator/src/learning/privacy.rs` as of commit e3bdf449f (2026-10-14), itself a
//! copy of the desktop app's `src-tauri/memory/src/privacy.rs` (b53f8ca0). When core changes
//! that file, change this one in the same release; `the_patterns_are_core_s` pins them, so an
//! edit here that drifts from core fails until the pin is moved on purpose.
//!
//! Core's `refuse_naming_categories` (for the one reviewed root line that names the categories)
//! is not copied: no line of the catalogue names them, and every line is sent with
//! `names_categories: false`, so every line meets the full filter.
//!
//! A text this refuses is never logged or echoed: callers say which line and why, not what.

use std::sync::LazyLock;

use regex::Regex;

/// Classified material: tenders, investors, banks, finance, contracts (English and Croatian).
pub(crate) const CLASSIFIED: &str = r"(?ix)
        \b(
            tenders? | tendering | natje[cč]aj\w* | javn\w*\s+nabav\w* | procurement | bid\s*bond | rfq | rfp
          | investors? | investments? | invest(ed|ing|s)? | invest(ir|itor|icij)\w* | ula[gž]a\w* | ulaganj\w* | dionic\w* | dioni[cč]ar\w* | udjel\w* | term\s*sheet | cap\s*table | valuation | shareholders? | equity
          | funding | fundrais\w* | seed\s*round | series\s*[ab] | pitch\s*deck | due\s*diligence | angel
          | banks? | banking | iban | swift\s*code | bic | loan | credit\s*line | overdraft
          | invoices? | revenue | profit | ebitda | salary | salaries | payroll | taxes | tax\s*return | vat\s*number
          | finances? | finance[-_.]\w* | eporezn\w* | bank_\w* | oib | \d{11} | balance\s*sheet | p\s*&\s*l | cash\s*flow | runway | burn\s*rate | accounting | bookkeeping | company\s*financ\w* | financials?
          | contracts? | ugovor\w* | addendum | nda\s*with
          | kredit\w* | porez\w* | prihod\w* | dobit | pla[ćc]a | plaće | plaću | plaćanj\w* | faktur\w* | ra[cč]unovod\w* | banc\w* | bank[aeiu]\w* | nabav\w* | ponud\w* | financij\w*
          | classified | confidential
        )\b";

/// Code words that only look like the classified ones: an API contract, a contract test.
pub(crate) const BENIGN: &str = r"(?ix)\b( (api|data|page|type|interface|wire|schema|tactical|console|smart|token|erc\d*|solidity|evm|deployed|response|request|rpc|proto|service|module|ui|design)[\s_-]contracts? | contracts?[\s_-](address(es)?|calls?|abi|bytecode|code|deploy\w*|storage|events?|tests?|checks?|files?) | contracts?[\s_-](tests?|checks?|files?\.?) | tactical_contract(\.py)? | rfc\s*0082-console-contract )\b";

/// Sums of money.
pub(crate) const MONEY: &str = r"(?i)(\d[\d.,]*\s*(k|m|mil|milijun\w*|million\w*|thousand|tisu[cć]\w*)?\s*(€|eur\b|eura\b|euro\w*|usd\b|dollars?\b|kn\b|kuna\b)|[€$£]\s*\d|\b(eur|usd|euro)\s*\(?\s*\d|\b(milijun|million|billion|milijard)\w*\s+(eura|euro|dollars?|usd|eur)\b)";

/// A reference to the owner's private vault.
pub(crate) const VAULT: &str =
    r"(?i)(\.inorbit/private|iohr-private|private\s+vault|vault\s+item|/private/drafts)";

/// The shapes of keys, tokens and passwords.
pub(crate) const SECRET: &str = r"(?x)
        sk-ant-[A-Za-z0-9_-]{8,} | sk-[A-Za-z0-9]{20,} | gh[pousr]_[A-Za-z0-9]{20,} | github_pat_[A-Za-z0-9_]{20,}
      | AKIA[0-9A-Z]{16} | xox[abpr]-[A-Za-z0-9-]{10,} | -----BEGIN[ A-Z]*PRIVATE\ KEY----- | eyJ[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]{10,}
      | (?i:(password|passwd|secret|api[_-]?key|token)\s*[:=]\s*\S{6,})";

/// The name of the reviewed rule that keeps the categories out: a citation may name it.
const CATEGORIES_RULE: &str = "classified-stays-with-owner";

fn re(p: &str) -> Regex {
    Regex::new(p).unwrap_or_else(|_| unreachable!("a fixed pattern, compiled in the tests"))
}

static CLASSIFIED_RE: LazyLock<Regex> = LazyLock::new(|| re(CLASSIFIED));
static BENIGN_RE: LazyLock<Regex> = LazyLock::new(|| re(BENIGN));
static MONEY_RE: LazyLock<Regex> = LazyLock::new(|| re(MONEY));
static VAULT_RE: LazyLock<Regex> = LazyLock::new(|| re(VAULT));
static SECRET_RE: LazyLock<Regex> = LazyLock::new(|| re(SECRET));

/// Whether `text` holds something shaped like a secret: a key, a token, a private key block,
/// `password: ...`.
#[must_use]
pub(crate) fn has_secret(text: &str) -> bool {
    SECRET_RE.is_match(text)
}

/// Why the service would refuse `text`, or `None` when it would keep it: `private-vault`,
/// `secret` or `classified`.
#[must_use]
pub(crate) fn refuse(text: &str) -> Option<&'static str> {
    if VAULT_RE.is_match(text) {
        return Some("private-vault");
    }
    if has_secret(text) {
        return Some("secret");
    }
    let without = BENIGN_RE.replace_all(text, " ");
    (CLASSIFIED_RE.is_match(&without) || MONEY_RE.is_match(&without)).then_some("classified")
}

/// [`refuse`] for a citation: only the reviewed rule's own name is let through.
#[must_use]
pub(crate) fn refuse_citation(cite: &str) -> Option<&'static str> {
    refuse(&cite.replace(CATEGORIES_RULE, " "))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A digest of every pattern. It moves only when a pattern does: then compare with core's
    /// `crates/coordinator/src/learning/privacy.rs` and move the commit in the module doc.
    #[test]
    fn the_patterns_are_core_s() {
        let all = [CLASSIFIED, BENIGN, MONEY, VAULT, SECRET, CATEGORIES_RULE].join("\u{0}");
        let digest = crate::memory::text::fnv(&all);
        assert_eq!(
            digest, PINNED,
            "a privacy pattern changed (digest {digest:#010x}): check it against core's privacy.rs"
        );
        for p in [CLASSIFIED, BENIGN, MONEY, VAULT, SECRET] {
            assert!(Regex::new(p).is_ok(), "{p}");
        }
    }

    /// Taken when every pattern was compared byte for byte with core e3bdf449f.
    const PINNED: u32 = 0x2b0e_1a91;

    #[test]
    fn owner_only_material_is_refused_as_the_service_refuses_it() {
        for t in [
            "The investors want the cap table updated",
            "Send the signed contract to the client",
            "Our revenue last quarter was flat",
            "We charge 49 EUR a month per seat",
            "the round is €2M",
            "a confidential note",
            "DORA (applying to financial entities since 17 January 2025)",
        ] {
            assert_eq!(refuse(t), Some("classified"), "{t}");
        }
        assert_eq!(
            refuse("see ~/.inorbit/private/drafts/x.md"),
            Some("private-vault")
        );
        assert_eq!(
            refuse("the key is sk-ant-api03-abcdefghijkl"),
            Some("secret")
        );
        assert_eq!(refuse("password = hunter22"), Some("secret"));
        assert_eq!(refuse("AKIAABCDEFGHIJKLMNOP"), Some("secret"));
        let names = "Tender documents, company finance and investors stay with the owner";
        assert_eq!(refuse(names), Some("classified"));
        assert_eq!(refuse_citation("file:classified-stays-with-owner"), None);
        assert_eq!(
            refuse_citation("file:confidential-investors-memo"),
            Some("classified")
        );
    }

    #[test]
    fn ordinary_work_and_code_words_pass() {
        for t in [
            "Never add a Co-Authored-By line to a commit",
            "The API contract for /v1/memory stays the same",
            "run the contract tests before merging",
            "keep the token budget under 1500",
        ] {
            assert_eq!(refuse(t), None, "{t}");
        }
    }
}
