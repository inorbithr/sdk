// Writes classified.json: the platform's TypeScript (ui/shared/lab in inorbithr/core)
// run over cases mirroring its ui/www/tool/classified.test.ts, for iohr-lab to match.
// Node 24 or later (it strips the types), from this directory:
//
//   node --no-warnings classified.gen.ts <core checkout> <this repo> classified.json
//
// Then update the commit named in the note.
import { readFileSync, writeFileSync } from "node:fs";
const core = process.argv[2];
const sdk = process.argv[3];
const { cut, parseClassified } = await import(`${core}/ui/shared/lab/classified.ts`);
const { checkDocument } = await import(`${core}/ui/shared/lab/check.ts`);

const SECRET = "zq-canary-7731";
const inline = (level: string, body = SECRET, reason = "the internal port") =>
  `[[classified:${level} reason="${reason}"]]${body}[[/classified]]`;
const block = (level: string, body = `${SECRET}\nsecond line`, reason = "the routing table", fence = "```") =>
  `${fence}classified level=${level} reason="${reason}"\n${body}\n${fence}`;

const cases: Record<string, string> = {
  "inline": `the balancer answers on ${inline("partner")} behind the edge`,
  "admin alias": inline("admin"),
  "block": `before\n${block("team")}\nafter`,
  "indented four-backtick block holding a fence": `- item\n  ${block("internal", "```sh\nrun " + SECRET + "\n```", "the command", "````").replace(/\n/g, "\n  ")}`,
  "multi-line inline": `a ${inline("partner", `${SECRET}\ncontinued`)} b`,
  "adjacent spans": `${inline("partner", "aaaa1")}${inline("internal", "bbbb2", "two")} and ${inline("preview", "cccc3", "three")}`,
  "every shape": [
    inline("preview"),
    inline("partner", `${SECRET}-b`),
    inline("team", `x ${SECRET}-c\ny`),
    block("internal"),
    block("admin", `${SECRET}-d`),
  ].join("\n\n"),
  "markers in a code fence are read too": "```md\n" + inline("team") + "\n```",
  "uppercase block keyword": '```Classified level=team reason="r"\nx\n```',
  "reason of 160": inline("team", SECRET, "r".repeat(160)),
  "no markers": "plain [REDACTED: an old marker]\n```redacted\nwhy\n```\n",
  "empty text": "",
  // the bad ones, as in the TS test
  "unclosed inline": `a [[classified:partner reason="r"]]${SECRET}`,
  "unclosed block": `${block("partner")}`.replace(/\n```$/, ""),
  "nested inline": `[[classified:partner reason="r"]]a ${inline("internal")} b[[/classified]]`,
  "inline inside a block": `\`\`\`classified level=partner reason="r"\n${inline("internal")}\n\`\`\``,
  "stray close": `text[[/classified]]`,
  "unknown level": `[[classified:secret reason="r"]]${SECRET}[[/classified]]`,
  "public is no span level": `[[classified:public reason="r"]]${SECRET}[[/classified]]`,
  "no reason": `[[classified:partner]]${SECRET}[[/classified]]`,
  "empty reason": `[[classified:partner reason=""]]${SECRET}[[/classified]]`,
  "] in a reason": `[[classified:partner reason="a ] b"]]${SECRET}[[/classified]]`,
  "spelling": `[[Classified:partner reason="r"]]${SECRET}[[/classified]]`,
  "spacing": `[[ classified:partner reason="r"]]${SECRET}[[/classified]]`,
  "empty span": `[[classified:partner reason="r"]][[/classified]]`,
  "across a paragraph": `[[classified:partner reason="r"]]a\n\nb[[/classified]]`,
  "block without a reason": "```classified level=partner\nx\n```",
  "tildes": '~~~classified level=partner reason="r"\nx\n~~~',
  "empty block": '```classified level=partner reason="r"\n```',
  "unclosed never leaks what follows": `ok [[classified:partner reason="r"]]${SECRET}\n\nmore`,
  "more: a quote in a reason": `[[classified:partner reason="a " b"]]${SECRET}[[/classified]]`,
  "more: a backtick in a reason": "[[classified:partner reason=\"a ` b\"]]" + SECRET + "[[/classified]]",
  "more: a reason of 161": inline("team", SECRET, "r".repeat(161)),
  "more: a blank reason": inline("team", SECRET, "   "),
  "more: an uppercase level": inline("PARTNER"),
  "more: a block inside a block": `${block("team", "```classified level=internal reason=\"x\"\ninner\n```")}`,
  "more: a block inside an inline span": `[[classified:team reason="r"]]a\n${block("internal")}\nb[[/classified]]`,
  "more: a tilde block inside a block": block("team", '~~~classified level=team reason="r"'),
  "more: a shorter fence does not close": block("team", "body\n```\nstill body", "r", "````"),
  "more: stray close after a close": `${inline("team")}[[/classified]]`,
  "more: unknown block level": block("secret"),
  "more: block attributes in the wrong order": '```classified reason="r" level=team\nx\n```',
  "more: whitespace-only block": '```classified level=team reason="r"\n  \n```',
  // A line ends at \n with or without a \r before it (CRLF read as LF; never an ordinary fence).
  "crlf: inline": `the balancer answers on ${inline("partner")} behind the edge\r\nnext line\r\n`,
  "crlf: block": `before\r\n${block("team").replace(/\n/g, "\r\n")}\r\nafter`,
  "crlf: unclosed block": `before\r\n${block("team").replace(/\n```$/, "").replace(/\n/g, "\r\n")}\r\n`,
  "crlf: multi-line inline": `a ${inline("partner", `${SECRET}\r\ncontinued`)} b\r\n`,
  "crlf: inline across a blank line": `[[classified:partner reason="r"]]a\r\n\r\nb[[/classified]]`,
  "crlf: a lone trailing carriage return": `${block("internal", SECRET, "r").replace(/\n/g, "\r\n")}\r`,
  "crlf: a carriage return inside a block reason": block("team", SECRET, "a\rb").replace(/\n/g, "\r\n"),
};

const out: any = {
  note: "Cases for classified spans, produced by running the platform's TypeScript (ui/shared/lab/classified.ts and check.ts at core commit 2e490130, inorbithr/core#409, RFC 0065 ladder) over inputs mirroring ui/www/tool/classified.test.ts and a few more. `cut` holds, per clearance, the cut text or null when cut refuses (problems); `problems` the problem lines; `spans` each span's kind, level and lines. `documents` hold check findings by rule and line, checked with spec/lab/rules.json and the conformance bundle's config. To move into the platform's docs/lab/conformance and arrive through spec:sync.",
  cases: {},
  documents: {},
};
const levels = ["public", "preview", "partner", "team", "internal"];
for (const [name, text] of Object.entries(cases)) {
  const p = parseClassified(text);
  const c: any = {};
  for (const l of levels) {
    try { c[l] = cut(text, l); } catch { c[l] = null; }
  }
  out.cases[name] = {
    text,
    problems: p.problems.map((x: any) => x.line),
    spans: p.spans.map((s: any) => ({ kind: s.kind, level: s.level, line: s.line, end_line: s.endLine })),
    cut: c,
  };
}

const bundle = JSON.parse(readFileSync(`${sdk}/spec/lab/conformance.json`, "utf8"));
const rules = JSON.parse(readFileSync(`${sdk}/spec/lab/rules.json`, "utf8")).rules;
const config = { ...bundle.config, rules };
const front = (extra = "", pub = "true") =>
  `---\ntitle: A classified document\nstatus: open\ndate: 2026-10-06\npublic: ${pub}\nlab: core\nsummary: One sentence.\n${extra}---\n`;
const docs: Record<string, string> = {
  "0001-withheld-text-is-exempt.md": `${front("reviewed: 2026-10-06\nreviewer: the owner\n")}\n## Problem\n\nThe primary is ${inline("internal", "db-primary at 10.0.0.12:5432")} today.\n\n${block("team", "deploy/ on api.example.com\nVaultPath")}\n\nAfter the block, db-primary is named in the open.\n`,
  "0002-a-reason-is-scanned.md": `${front()}\n## Problem\n\nThe host is ${inline("partner", "x", "the name api.example.com")}.\n\n${block("team", "x", "the db-primary table")}\n`,
  "0003-a-bad-marker-reads-the-text-as-it-is.md": `${front()}\n## Problem\n\nThe primary is [[classified:secret reason="r"]]db-primary[[/classified]].\n`,
  "0004-front-matter-carries-no-span.md": `${front(`headline: ${inline("partner", "x")}\n`)}\n## Problem\n\nPlain.\n`,
  "0005-reviewed-must-be-a-date.md": `${front("reviewer: someone\nreviewed: yesterday\n")}\n## Problem\n\nPlain.\n`,
  "0006-a-draft-is-checked-for-markers.md": `${front("", "false")}\n## Problem\n\n~~~classified level=team reason="r"\ndb-primary\n~~~\n\n${inline("team", "")}\n`,
  "0007-lines-after-a-withheld-block.md": `${front()}\n## Problem\n\n${block("team", "a\nb\nc\nd")}\n\nThen db-primary, on its own line.\n\n## Status log\n\n- 2026-10-06: opened.\n- not a date\n`,
};
for (const [name, text] of Object.entries(docs)) {
  const r = checkDocument(name, text, "rfc", config);
  out.documents[`rfcs/${name}`] = {
    text,
    expected: r.findings.map((f: any) => ({ line: f.line, rule: f.rule })),
  };
}
writeFileSync(process.argv[4], JSON.stringify(out, null, 2) + "\n");
console.log(Object.keys(out.cases).length, "cases,", Object.keys(out.documents).length, "documents");
for (const [k, v] of Object.entries(out.documents) as any) console.log(k, JSON.stringify(v.expected));
