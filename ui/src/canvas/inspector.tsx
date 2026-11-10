// The canvas dock (frame/dock.tsx, reused): the details of the one selected item, with the same edits as the menu.
import { Dock } from "../frame/dock.tsx";
import { TONES, type Item, type Tone } from "./schema.ts";

export function Inspector({
  item,
  width,
  onWidth,
  onClose,
  onTitle,
  onText,
  onTone,
}: {
  item: Item;
  width: number;
  onWidth: (w: number) => void;
  onClose: () => void;
  onTitle: (t: string) => void;
  onText: (t: string) => void;
  onTone: (t: Tone) => void;
}) {
  const text = item.type === "note" || item.type === "frame" || item.type === "shape" ? (item.text ?? "") : null;
  const tone = item.type === "note" || item.type === "frame" ? (item.tone ?? "plain") : null;
  return (
    <Dock title="Details" width={width} onWidth={onWidth} onClose={onClose}>
      <div className="flex flex-col gap-3 overflow-auto p-4 text-sm">
        <p className="text-muted-foreground text-[12px]">
          {item.type}, {Math.round(item.w)} by {Math.round(item.h)} at {Math.round(item.x)}, {Math.round(item.y)}
        </p>
        {item.type !== "note" ? (
          <label className="flex flex-col gap-1">
            <span className="text-muted-foreground text-[12px]">Title</span>
            <input className="bg-background border-input rounded-md border px-2 py-1" value={item.title} onChange={(e) => onTitle(e.target.value)} />
          </label>
        ) : null}
        {text !== null ? (
          <label className="flex flex-col gap-1">
            <span className="text-muted-foreground text-[12px]">Text</span>
            <textarea className="bg-background border-input min-h-24 rounded-md border px-2 py-1" value={text} onChange={(e) => onText(e.target.value)} />
          </label>
        ) : null}
        {tone !== null ? (
          <fieldset className="flex flex-wrap gap-1">
            <legend className="text-muted-foreground mb-1 text-[12px]">Colour</legend>
            {TONES.map((t) => (
              <button key={t} type="button" aria-pressed={t === tone} onClick={() => onTone(t)} className={`rounded-md border px-2 py-0.5 text-[12px] ${t === tone ? "bg-accent" : ""}`}>
                {t}
              </button>
            ))}
          </fieldset>
        ) : null}
      </div>
    </Dock>
  );
}
