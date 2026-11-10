// Recent sessions on a planet (PRD 0117 slice D). Your own show in full; someone else's show only
// that one is running, with "Ask to view", which asks the coordinator to put the request to them.

import { useState } from "react";
import { Button, Tag } from "../components/kit.tsx";
import { agoText, redact, type SessionPreview } from "../lib/sensors-core.ts";
import { askToView } from "./actions.ts";

function Row({ s, now }: { s: SessionPreview; now: number }) {
  const shown = redact(s);
  const [asked, setAsked] = useState<{ ok: boolean; text: string } | null>(null);
  const [busy, setBusy] = useState(false);
  if (!s.mine)
    return (
      <li className="flex flex-wrap items-center gap-x-3 gap-y-1 py-2.5">
        <p className="min-w-0 flex-1 text-[13px]">{shown.title}</p>
        {!s.thread_id && <span className="text-muted-foreground text-[12px]">Only its owner can open it</span>}
        {s.thread_id && !asked?.ok && (
          <Button
            size="sm"
            variant="outline"
            disabled={busy}
            onClick={async () => {
              setBusy(true);
              setAsked(await askToView(s.thread_id));
              setBusy(false);
            }}
          >
            {busy ? "Asking…" : "Ask to view"}
          </Button>
        )}
        {asked && (
          <p
            role="status"
            className={asked.ok ? "text-success w-full text-[12px]" : "text-muted-foreground w-full text-[12px]"}
          >
            {asked.text}
          </p>
        )}
      </li>
    );
  return (
    <li className="py-2.5">
      <div className="flex items-baseline gap-2">
        <p className="min-w-0 flex-1 truncate text-[13px] font-medium">{s.title}</p>
        <Tag>{s.state}</Tag>
      </div>
      <p className="text-muted-foreground mt-0.5 truncate text-[12px]">
        {s.last_step ? `${s.last_step} · ` : ""}
        {s.kind === "ask" ? "your ask" : "yours"}, {agoText(s.at, now)}
      </p>
    </li>
  );
}

export function SessionList({
  sessions,
  now,
  empty,
}: {
  sessions: readonly SessionPreview[];
  now: number;
  empty: string;
}) {
  if (!sessions.length) return <p className="text-muted-foreground py-2 text-[12.5px]">{empty}</p>;
  return (
    <ul className="divide-y">
      {sessions.map((s) => (
        <Row key={s.id} s={s} now={now} />
      ))}
    </ul>
  );
}
