// biome-ignore-all lint/suspicious/noArrayIndexKey: the parsed blocks are drawn once and never reordered.
// Draws what `parseMarkdown` made as React elements (never an HTML string).
import type { ReactNode } from "react";
import { type Block, type Inline, parseMarkdown } from "../lib/markdown-core.ts";

const runs = (v: Inline[]): ReactNode[] =>
  v.map((x, i) => {
    const key = `${i}`;
    switch (x.t) {
      case "text":
        return x.v;
      case "code":
        return (
          <code key={key} className="bg-muted rounded px-1 py-0.5 text-[0.9em]">
            {x.v}
          </code>
        );
      case "strong":
        return <strong key={key}>{runs(x.v)}</strong>;
      case "em":
        return <em key={key}>{runs(x.v)}</em>;
      case "link":
        return (
          <a
            key={key}
            href={x.href}
            className="text-primary underline underline-offset-2"
            {...(x.href.startsWith("#") ? {} : { target: "_blank", rel: "noopener noreferrer" })}
          >
            {runs(x.v)}
          </a>
        );
      default:
        return null;
    }
  });

const SIZE = ["", "text-2xl", "text-xl", "text-lg", "text-base", "text-sm", "text-sm"];

function block(b: Block, i: number): ReactNode {
  switch (b.t) {
    case "heading": {
      const Tag = `h${Math.min(b.level, 6)}` as "h1";
      return (
        <Tag key={i} id={b.id} className={`mt-6 mb-2 scroll-mt-4 font-semibold ${SIZE[b.level]}`}>
          {runs(b.v)}
        </Tag>
      );
    }
    case "para":
      return (
        <p key={i} className="my-3 leading-relaxed">
          {runs(b.v)}
        </p>
      );
    case "code":
      return (
        <pre key={i} className="bg-muted my-3 overflow-x-auto rounded-lg p-3 text-[12px]" data-lang={b.lang}>
          <code>{b.v}</code>
        </pre>
      );
    case "quote":
      return (
        <blockquote key={i} className="text-muted-foreground my-3 border-l-2 pl-3">
          {b.v.map(block)}
        </blockquote>
      );
    case "list": {
      const Tag = b.ordered ? "ol" : "ul";
      return (
        <Tag key={i} className={`my-3 pl-5 ${b.ordered ? "list-decimal" : "list-disc"}`}>
          {b.items.map((it, j) => (
            <li key={`${i}-${j}`} className="my-1">
              {runs(it)}
            </li>
          ))}
        </Tag>
      );
    }
    case "table":
      return (
        <div key={i} className="my-3 overflow-x-auto">
          <table className="w-full border-collapse text-sm">
            <thead>
              <tr>
                {b.head.map((c, j) => (
                  <th key={`h${j}`} className="border-b px-2 py-1 text-left font-medium">
                    {runs(c)}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {b.rows.map((r, j) => (
                <tr key={`r${j}`}>
                  {r.map((c, k) => (
                    <td key={`c${k}`} className="border-b px-2 py-1">
                      {runs(c)}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    case "rule":
      return <hr key={i} className="my-4" />;
  }
}

export function MarkdownView({ text }: { text: string }) {
  const { front, blocks } = parseMarkdown(text);
  return (
    <div className="selectable text-sm">
      {front.length > 0 && (
        <dl className="bg-muted/40 mb-4 grid grid-cols-[max-content_1fr] gap-x-4 gap-y-0.5 rounded-lg border p-3 text-[12px]">
          {front.map(([k, v]) => (
            <div key={k} className="contents">
              <dt className="text-muted-foreground">{k}</dt>
              <dd>{v}</dd>
            </div>
          ))}
        </dl>
      )}
      {blocks.map(block)}
    </div>
  );
}
