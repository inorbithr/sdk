// The small parts the Mac app's kit gives its pages, for the sdk's pages: a button, a tag, a card, a loading line
// and a problems list, drawn from the theme tokens (frame/tokens.css).
import type { ButtonHTMLAttributes, HTMLAttributes, ReactNode } from "react";
import { cn } from "../frame/cn.ts";

export { cn };

type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: "default" | "outline" | "ghost" | "destructive";
  size?: "sm" | "md";
};

export function Button({ variant = "default", size = "md", className, type = "button", ...rest }: ButtonProps) {
  return (
    <button
      type={type}
      className={cn(
        "focus-visible:ring-ring/50 inline-flex shrink-0 items-center justify-center gap-1.5 whitespace-nowrap rounded-md font-medium outline-none transition-colors focus-visible:ring-[3px] disabled:pointer-events-none disabled:opacity-50 [&_svg]:size-3.5",
        size === "sm" ? "h-7 px-2.5 text-[12.5px]" : "h-9 px-3.5 text-[13.5px]",
        variant === "default" && "bg-primary text-primary-foreground hover:bg-primary/90",
        variant === "outline" && "bg-background hover:bg-accent hover:text-accent-foreground border shadow-xs",
        variant === "ghost" && "hover:bg-accent hover:text-accent-foreground",
        variant === "destructive" && "bg-destructive hover:bg-destructive/90 text-white",
        className,
      )}
      {...rest}
    />
  );
}

export const Tag = ({ className, children }: { className?: string; children: ReactNode }) => (
  <span className={cn("inline-flex items-center rounded-md border px-1.5 py-0.5 text-[11.5px] font-medium", className)}>
    {children}
  </span>
);

type CardProps = Omit<HTMLAttributes<HTMLDivElement>, "title"> & {
  title?: ReactNode;
  description?: ReactNode;
  aside?: ReactNode;
  bodyClass?: string;
};

export const Card = ({ className, children, title, description, aside, bodyClass, ...rest }: CardProps) => (
  <div className={cn("bg-card text-card-foreground rounded-2xl border shadow-xs", className)} {...rest}>
    {title != null && (
      <div className="flex items-start gap-3 px-5 pt-4">
        <div className="min-w-0 flex-1">
          <h3 className="text-[14px] leading-snug font-semibold">{title}</h3>
          {description && <p className="text-muted-foreground mt-0.5 text-[12.5px] text-pretty">{description}</p>}
        </div>
        {aside}
      </div>
    )}
    <div className={cn("px-5", title != null ? "pt-3 pb-4" : "py-4", bodyClass)}>{children}</div>
  </div>
);

export const Loading = ({ what }: { what: string }) => (
  <p role="status" className="text-muted-foreground rounded-xl border px-6 py-10 text-center text-[13.5px]">
    {what}
  </p>
);

export const Problems = ({ errors }: { errors: readonly string[] }) => (
  <ul role="alert" className="border-destructive/40 text-destructive space-y-1 rounded-xl border px-4 py-3 text-[12.5px]">
    {errors.map((e) => (
      <li key={e}>{e}</li>
    ))}
  </ul>
);
