import { ChevronsUpDown, LogIn } from "lucide-react";
import { cn } from "./frame/cn.ts";
import type { SessionState } from "./lib/api.ts";

// Who this page acts as, and the sign-in (RFC 0217, round 4). Sign-in is `iohr login --web`: the person's browser opens
// and the local `iohr ui serve` keeps the credential, never this page. Signed in, the header shows the account's name
// and opens the account switcher (RFC 0217.1), which also holds Sign out; it never shows an id.
const button =
  "hover:bg-accent hover:text-foreground text-muted-foreground flex items-center gap-1.5 whitespace-nowrap rounded-md px-2 py-1 text-[13px] disabled:opacity-50 [&>svg]:size-3.5";

export function SessionControl({
  session,
  name,
  busy,
  onSignIn,
  onOpen,
}: {
  session: SessionState | null;
  /** The account's name (`accountName`), never an id. */
  name: string;
  busy: boolean;
  onSignIn: () => void;
  onOpen: (button: HTMLElement) => void;
}) {
  if (session === null) return <span className="text-muted-foreground px-2 text-[13px]">Checking sign-in…</span>;
  if (session.state === "unreachable")
    return (
      <span className="text-muted-foreground max-w-56 truncate px-2 text-[13px]" role="alert" title={session.reason}>
        Not connected
      </span>
    );
  if (session.state === "signed-in")
    return (
      <button
        type="button"
        aria-label={`Account: ${name}`}
        aria-haspopup="menu"
        data-account-button
        title={`${name} · ${session.profile}`}
        className={cn(button, "max-w-56")}
        onClick={(e) => onOpen(e.currentTarget)}
      >
        <span className="truncate @max-[640px]:hidden">{name}</span>
        <ChevronsUpDown aria-hidden="true" />
      </button>
    );
  const waiting = session.login === "waiting";
  return (
    <div className="flex items-center gap-1">
      {session.login === "failed" ? (
        <span className="text-destructive max-w-56 truncate px-2 text-[13px]" role="alert" title={session.reason ?? ""}>
          Sign-in failed
        </span>
      ) : null}
      <button
        type="button"
        aria-label={waiting ? "Finish in your browser" : "Sign in"}
        className={cn(button, "text-foreground")}
        disabled={busy || waiting}
        onClick={onSignIn}
      >
        <LogIn aria-hidden="true" />
        <span className="@max-[640px]:hidden">{waiting ? "Finish in your browser…" : "Sign in"}</span>
      </button>
    </div>
  );
}

/** The rail's footer, as the Mac app's avatar at the bottom of the rail: who the page acts as, nothing else. */
export function RailAccount({
  session,
  name: accountName,
  rail,
  onOpen,
}: {
  session: SessionState | null;
  name: string;
  rail: boolean;
  onOpen: (button: HTMLElement) => void;
}) {
  const name = session?.state === "signed-in" ? accountName : null;
  const label =
    name ??
    (session === null ? "Checking sign-in…" : session.state === "unreachable" ? "Not connected" : "Not signed in");
  const initial = name ? name.trim().charAt(0).toUpperCase() : "?";
  const Tag = name === null ? "div" : "button";
  return (
    <Tag
      {...(name === null
        ? {}
        : {
            type: "button" as const,
            "aria-haspopup": "menu" as const,
            onClick: (e: { currentTarget: HTMLElement }) => onOpen(e.currentTarget),
          })}
      className={cn(
        "flex items-center gap-2 text-[13px]",
        rail && "justify-center",
        name !== null && "hover:bg-accent w-full rounded-md p-1 text-left",
      )}
      title={label}
      data-account={name === null ? "none" : "signed-in"}
    >
      <span
        aria-hidden="true"
        className={cn(
          "flex size-7 shrink-0 items-center justify-center rounded-full text-[12px] font-medium",
          name ? "bg-primary text-primary-foreground" : "bg-muted text-muted-foreground",
        )}
      >
        {initial}
      </span>
      {rail ? <span className="sr-only">{label}</span> : <span className="min-w-0 flex-1 truncate">{label}</span>}
    </Tag>
  );
}
