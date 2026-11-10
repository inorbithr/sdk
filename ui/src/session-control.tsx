import { LogIn, LogOut } from "lucide-react";
import { cn } from "./frame/cn.ts";
import type { SessionState } from "./lib/api.ts";

// Who this page acts as, and the sign-in and sign-out (RFC 0217, round 4). Sign-in is `iohr login --web`: the person's
// browser opens and the local `iohr ui serve` keeps the credential, never this page.
const button =
  "hover:bg-accent hover:text-foreground text-muted-foreground flex items-center gap-1.5 whitespace-nowrap rounded-md px-2 py-1 text-[13px] disabled:opacity-50 [&>svg]:size-3.5";

export function SessionControl({
  session,
  busy,
  onSignIn,
  onSignOut,
}: {
  session: SessionState | null;
  busy: boolean;
  onSignIn: () => void;
  onSignOut: () => void;
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
      <div className="flex items-center gap-1">
        <span
          className="text-muted-foreground max-w-48 truncate px-2 text-[13px] @max-[640px]:hidden"
          title={`${session.profile} · ${session.subject}`}
        >
          {session.account === "" ? session.subject : session.account}
        </span>
        <button type="button" aria-label="Sign out" className={button} disabled={busy} onClick={onSignOut}>
          <LogOut aria-hidden="true" />
          <span className="@max-[640px]:hidden">Sign out</span>
        </button>
      </div>
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
export function RailAccount({ session, rail }: { session: SessionState | null; rail: boolean }) {
  const name = session?.state === "signed-in" ? (session.account === "" ? session.subject : session.account) : null;
  const label =
    name ??
    (session === null ? "Checking sign-in…" : session.state === "unreachable" ? "Not connected" : "Not signed in");
  const initial = name ? name.trim().charAt(0).toUpperCase() : "?";
  return (
    <div
      className={cn("flex items-center gap-2 text-[13px]", rail && "justify-center")}
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
    </div>
  );
}
