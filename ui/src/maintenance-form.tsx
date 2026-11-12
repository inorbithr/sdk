import { useEffect, useRef, useState } from "react";
import { featureOf } from "./lib/features-core.ts";

// The form that puts a page in maintenance: an optional reason and an optional "back at", both shown on the page while
// it is down. The state is kept in this browser (lib/features-store.ts); it does not stop anything on a server. Escape
// and Cancel leave without changing anything, and Enter in either field saves.
export function MaintenanceForm({
  id,
  onSave,
  onClose,
}: {
  id: string;
  onSave: (why: { reason: string; eta: string }) => void;
  onClose: () => void;
}) {
  const [reason, setReason] = useState("");
  const [eta, setEta] = useState("");
  const first = useRef<HTMLInputElement>(null);
  const name = featureOf(id)?.name ?? id;
  useEffect(() => first.current?.focus(), []);
  const save = () => {
    onSave({ reason, eta });
    onClose();
  };
  const field =
    "bg-background w-full rounded-md border px-2 py-1.5 text-[13px] outline-none focus-visible:ring-2 ring-ring";
  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center bg-black/30 pt-[18vh]" data-maintenance-form>
      <div
        role="dialog"
        aria-modal="true"
        aria-label={`Put ${name} in maintenance`}
        className="bg-popover text-popover-foreground w-[min(92vw,380px)] rounded-xl border p-4 shadow-xl"
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            onClose();
          } else if (e.key === "Enter") {
            e.preventDefault();
            save();
          }
        }}
      >
        <h2 className="mb-1 text-sm font-medium">Put {name} in maintenance</h2>
        <p className="text-muted-foreground mb-3 text-[12px]">
          The page says it is under maintenance, with what you write here. This is kept in this browser; it does not
          stop anything on a server.
        </p>
        <label className="mb-3 flex flex-col gap-1 text-[12px]">
          Reason (optional)
          <input
            ref={first}
            className={field}
            value={reason}
            maxLength={200}
            onChange={(e) => setReason(e.target.value)}
          />
        </label>
        <label className="mb-4 flex flex-col gap-1 text-[12px]">
          Back at (optional)
          <input className={field} value={eta} maxLength={200} onChange={(e) => setEta(e.target.value)} />
        </label>
        <div className="flex justify-end gap-2 text-[13px]">
          <button type="button" className="hover:bg-accent rounded-md px-3 py-1.5" onClick={onClose}>
            Cancel
          </button>
          <button type="button" className="bg-primary text-primary-foreground rounded-md px-3 py-1.5" onClick={save}>
            Put in maintenance
          </button>
        </div>
      </div>
    </div>
  );
}
