// The board for one canvas id as a React hook (PRD 0065): a Board over this browser's storage, read through
// useSyncExternalStore. Another tab's save is read in (`storage` event); leaving the page saves what is pending.
import { useEffect, useState, useSyncExternalStore } from "react";
import { Board, BODY, browserKV, type Snapshot } from "./board-store-core.ts";

const none = () => () => {};
const empty: Snapshot = { canvas: null, error: null, saving: false, savedAt: null, past: 0, future: 0, dirty: false };
const nothing = () => empty;

export function useBoard(id: string): { board: Board | null; snap: Snapshot } {
  const [board, setBoard] = useState<Board | null>(null);
  useEffect(() => {
    const b = new Board(id, browserKV());
    setBoard(b);
    const onStorage = (e: StorageEvent) => {
      if (e.key === null || e.key === BODY + id) b.external();
    };
    const onHide = () => b.flush();
    window.addEventListener("storage", onStorage);
    window.addEventListener("pagehide", onHide);
    return () => {
      window.removeEventListener("storage", onStorage);
      window.removeEventListener("pagehide", onHide);
      b.dispose();
      setBoard(null);
    };
  }, [id]);
  const snap = useSyncExternalStore(board ? board.subscribe : none, board ? board.getSnapshot : nothing);
  return { board, snap };
}
