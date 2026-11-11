// Io as the author of a Decisions record (RFC 0065.2, section 5): one formatter for every
// surface that shows who wrote a document, a version, a comment or a review. A record
// Io wrote reads "Io (Security), asked by Nevio": the persona's name, the personality the
// record's `via` names, and the person whose ask started the work. A record without
// `via` by Io reads "Io"; a person's record is left to the page's own name resolver.
//
// The source lives in ui/shared; `mise run ui:shared:sync` copies it to src/lib/io/via.ts
// of the console (the desktop app, its own repository, keeps a copy at src/lib/io-via.ts;
// the phone has the same rules in lib/app/persona.dart). It imports nothing so every
// copy is the file as is.

/** What a write records about the agent session behind it (labs.proto `Via`). */
export type Via = {
  /** The personality that acted: `security`, `ui-designer`, … */
  personality?: string;
  /** The Claude session or runner lease id. */
  session?: string;
  /** The coordinator job id, when there is one. */
  job?: string;
  /** The subject of the person whose ask or message started the work. */
  requested_by?: string;
};

/** The persona's name, the same on every surface (persona.ts). */
export const IO_NAME = "Io";

/**
 * The subjects Io's writes carry: the Io client (RFC 0065.2) and the RFCs agent it
 * replaced (RFC 0065.1), whose rows keep their old subject until they are reattributed.
 */
export const IO_SUBJECTS: readonly string[] = ["iohr-io", "iohr-rfcs-agent"];

/** True for a subject that is Io, not a person. */
export function isIoSubject(subject: string | null | undefined): boolean {
  return !!subject && IO_SUBJECTS.includes(subject);
}

/** Short words that read as initials: `ui-designer` is "UI designer", not "Ui designer". */
const INITIALISMS = new Set(["ui", "ux", "api", "ai", "ci", "qa", "sre", "seo"]);

/**
 * A personality's name as a title: `security` is "Security", `ui-designer` is "UI designer",
 * `unknown:foo` (a name the registry does not know) is "foo". Empty stays empty.
 */
export function personalityName(personality: string | null | undefined): string {
  const raw = (personality ?? "").trim().replace(/^unknown:/, "");
  if (!raw) return "";
  return raw
    .split(/[-_\s]+/)
    .filter(Boolean)
    .map((w, i) =>
      INITIALISMS.has(w.toLowerCase())
        ? w.toUpperCase()
        : i === 0
          ? w.charAt(0).toUpperCase() + w.slice(1)
          : w.toLowerCase(),
    )
    .join(" ");
}

/** Io as a record's author: the name, the personality (may be empty) and who asked (may be empty). */
export type IoAuthor = {
  name: string;
  personality: string;
  askedBy: string;
};

/**
 * Io when the record is Io's, else null. A record is Io's when its subject is an Io client,
 * or when its `via` names a personality (a write through an agent session on a backend that
 * still signs as the person, RFC 0065.2 section 6). `nameOf` turns `requested_by` into the
 * person's name the way the page names every other subject. A person-signed record always
 * names its signer: without `requested_by` the subject itself is who asked, so a person's
 * own write never reads as Io's alone.
 */
export function ioAuthor(
  subject: string | null | undefined,
  via: Via | null | undefined,
  nameOf: (subject: string) => string,
): IoAuthor | null {
  const personality = personalityName(via?.personality);
  if (!isIoSubject(subject) && !personality) return null;
  const requestedBy = via?.requested_by || (isIoSubject(subject) ? "" : (subject ?? ""));
  return { name: IO_NAME, personality, askedBy: requestedBy ? nameOf(requestedBy) : "" };
}

/**
 * The author as one line, for places that take text: "Io (Security), asked by Nevio".
 * `askedBy` says "asked by <who>" in the reader's language.
 */
export function ioAuthorText(a: IoAuthor, askedBy: (who: string) => string = (who) => `asked by ${who}`): string {
  let s = a.name;
  if (a.personality) s += ` (${a.personality})`;
  if (a.askedBy) s += `, ${askedBy(a.askedBy)}`;
  return s;
}
