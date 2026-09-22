import type { ReactNode } from "react";
import { clsx } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: Array<string | false | null | undefined>): string {
  return twMerge(clsx(...inputs));
}

export function Card({ className, children }: { className?: string; children: ReactNode }) {
  return (
    <div className={cn("min-w-0 rounded border border-rule bg-card p-4", className)}>{children}</div>
  );
}

export function CardTitle({ children }: { children: ReactNode }) {
  return <p className="m-0 text-xs uppercase tracking-wide text-muted">{children}</p>;
}

type Tone = "default" | "secondary" | "ok" | "bad" | "warn" | "info";

const tones: Record<Tone, string> = {
  default: "border-transparent bg-ink text-white",
  secondary: "border-transparent bg-rule text-ink",
  ok: "border-[#a9dcc0] bg-leafbg text-[#14703c]",
  bad: "border-transparent bg-blood text-white",
  warn: "border-[#f3d3ae] bg-emberbg text-[#b35a10]",
  info: "border-[#b8d6ec] bg-[#eef5fb] text-deep",
};

export function Badge({ tone = "secondary", className, children }: { tone?: Tone; children: ReactNode; className?: string }) {
  return (
    <span
      className={cn(
        "inline-flex items-center rounded-md border px-2.5 py-0.5 text-xs font-semibold tabular-nums transition-colors focus-visible:outline-2 focus-visible:outline-sky",
        tones[tone],
        className,
      )}
    >
      {children}
    </span>
  );
}

export function severityTone(severity: string | null): Tone {
  switch ((severity ?? "").toLowerCase()) {
    case "error":
      return "bad";
    case "warning":
      return "warn";
    case "info":
      return "info";
    default:
      return "default";
  }
}
