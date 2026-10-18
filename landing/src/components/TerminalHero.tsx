import { createSignal, onCleanup, onMount } from "solid-js";

const INSTALL =
  "curl -fsSL https://raw.githubusercontent.com/jbt95/leadline/main/install.sh | sh";

const SESSION: { type: "in" | "out"; text: string; class?: string }[] = [
  { type: "in", text: "$ leadline changed --base origin/main --format agent-json" },
  {
    type: "out",
    text: `{"summary":{"changed_functions":1,"regressions":1,"improvements":0},
 "regressions":[{"path":"payment.ts","function":"processPayment",
  "before":{"cognitive":12},"after":{"cognitive":24}}]}`,
    class: "text-danger",
  },
  { type: "in", text: "$ # agent refactors processPayment" },
  { type: "in", text: "$ leadline changed --base origin/main --format agent-json" },
  {
    type: "out",
    text: `{"summary":{"changed_functions":1,"regressions":0,"improvements":1},
 "regressions":[],
 "improvements":[{"path":"payment.ts","function":"processPayment",
  "before":{"cognitive":24},"after":{"cognitive":12}}]}`,
    class: "text-accent",
  },
];

/** Terminal window reproducing documented agent-json changed output (README). */
export function TerminalHero() {
  const [lines, setLines] = createSignal<{ type: "in" | "out"; text: string; class?: string }[]>(
    [],
  );
  const [cursor, setCursor] = createSignal(true);

  onMount(() => {
    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    if (reduced) {
      setLines(SESSION);
      setCursor(false);
      return;
    }

    let step = 0;
    let charIdx = 0;
    let current = "";
    let timeout: ReturnType<typeof setTimeout>;
    const blink = setInterval(() => setCursor((c) => !c), 530);

    const typeNext = () => {
      if (step >= SESSION.length) {
        timeout = setTimeout(() => {
          setLines([]);
          step = 0;
          charIdx = 0;
          typeNext();
        }, 4000);
        return;
      }
      const entry = SESSION[step];
      if (charIdx === 0 && entry.type === "out") {
        setLines((prev) => [...prev, { ...entry, text: "" }]);
      } else if (charIdx === 0) {
        setLines((prev) => [...prev, { ...entry, text: "" }]);
      }
      current += entry.text[charIdx] ?? "";
      charIdx++;
      setLines((prev) => {
        const next = [...prev];
        next[next.length - 1] = { ...entry, text: current };
        return next;
      });
      const delay = entry.type === "in" ? 28 : 6;
      if (charIdx >= entry.text.length) {
        step++;
        charIdx = 0;
        current = "";
        timeout = setTimeout(typeNext, entry.type === "out" ? 900 : 400);
      } else {
        timeout = setTimeout(typeNext, delay);
      }
    };

    timeout = setTimeout(typeNext, 600);
    onCleanup(() => {
      clearTimeout(timeout);
      clearInterval(blink);
    });
  });

  return (
    <div
      class="overflow-hidden rounded-lg border border-rule bg-panel shadow-[0_0_0_1px_rgba(61,214,140,0.06)]"
      role="region"
      aria-label="Terminal session: leadline changed agent-json"
    >
      <div class="flex items-center gap-2 border-b border-rule px-3 py-2 text-[10px] text-muted">
        <span class="h-2.5 w-2.5 rounded-full bg-rule-strong" aria-hidden="true" />
        <span class="h-2.5 w-2.5 rounded-full bg-rule-strong" aria-hidden="true" />
        <span class="h-2.5 w-2.5 rounded-full bg-rule-strong" aria-hidden="true" />
        <span class="ml-2 font-mono">leadline · zsh</span>
      </div>
      <div class="max-h-[min(320px,50vh)] overflow-x-auto overflow-y-auto p-4 font-mono text-[11px] leading-relaxed sm:text-xs">
        <p class="mb-3 text-muted">
          <span class="text-accent">$</span> {INSTALL}
        </p>
        {lines().map((line, i) => (
          <p class={`mb-2 whitespace-pre-wrap break-all ${line.class ?? "text-ink"}`}>
            {line.type === "in" ? (
              <>
                <span class="text-accent">$</span> {line.text.replace(/^\$ /, "")}
              </>
            ) : (
              line.text
            )}
          </p>
        ))}
        {cursor() && (
          <span class="inline-block h-3 w-2 bg-accent align-middle" aria-hidden="true" />
        )}
      </div>
    </div>
  );
}

export const installCommand = INSTALL;
