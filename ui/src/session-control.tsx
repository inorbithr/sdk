import { LogIn, LogOut } from "lucide-react";
import { cn } from "./frame/cn.ts";
import type { SessionState } from "./lib/api.ts";

// Who this page acts as, and the sign-in and sign-out (RFC 0217, round 4). Sign-in is `iohr login --web`: the person's
// browser opens and the local `iohr ui serve` keeps the credential, never this page.
const button =
  "hover:bg-accent hover:text-foreground text-muted-foreground flex items-center gap-1.5 rounded-md px-2 py-1 text-[13px] disabled:opacity-50 [&>svg]:size-3.5";

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
          className="text-muted-foreground max-w-48 truncate px-2 text-[13px]"
          title={`${session.profile} · ${session.subject}`}
        >
          {session.account === "" ? session.subject : session.account}
        </span>
        <button type="button" className={button} disabled={busy} onClick={onSignOut}>
          <LogOut aria-hidden="true" />
          Sign out
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
      <button type="button" className={cn(button, "text-foreground")} disabled={busy || waiting} onClick={onSignIn}>
        <LogIn aria-hidden="true" />
        {waiting ? "Finish in your browser…" : "Sign in"}
      </button>
    </div>
  );
}
