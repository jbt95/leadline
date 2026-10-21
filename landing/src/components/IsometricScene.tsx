import { createSignal, onCleanup, onMount } from "solid-js";

type Phase = "small" | "grow" | "scan" | "shrink" | "done";

const PHASE_MS: Record<Phase, number> = {
  small: 1200,
  grow: 1800,
  scan: 2200,
  shrink: 1600,
  done: 1400,
};

/** Isometric function blocks: complexity grows, leadline scans, fix restores size. */
export function IsometricScene() {
  const [phase, setPhase] = createSignal<Phase>("small");
  const [scanLine, setScanLine] = createSignal(0);
  let reduced = false;

  onMount(() => {
    reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    if (reduced) {
      setPhase("done");
      return;
    }

    let current: Phase = "small";
    let timeout: ReturnType<typeof setTimeout>;
    let raf = 0;
    let scanStart = 0;

    const runScan = () => {
      scanStart = performance.now();
      const tick = (now: number) => {
        const t = Math.min(1, (now - scanStart) / PHASE_MS.scan);
        setScanLine(t);
        if (t < 1) raf = requestAnimationFrame(tick);
      };
      raf = requestAnimationFrame(tick);
    };

    const next = () => {
      const order: Phase[] = ["small", "grow", "scan", "shrink", "done"];
      const idx = order.indexOf(current);
      current = order[(idx + 1) % order.length];
      setPhase(current);
      if (current === "scan") {
        setScanLine(0);
        runScan();
      }
      timeout = setTimeout(next, PHASE_MS[current]);
    };

    timeout = setTimeout(next, PHASE_MS.small);
    onCleanup(() => {
      clearTimeout(timeout);
      cancelAnimationFrame(raf);
    });
  });

  const height = () => {
    const p = phase();
    if (p === "small" || p === "done") return 28;
    if (p === "grow" || p === "scan") return 88;
    if (p === "shrink") return 36;
    return 28;
  };

  const cognitiveLabel = () => {
    const p = phase();
    if (p === "small" || p === "done") return "cognitive 12";
    if (p === "grow" || p === "scan") return "cognitive 24";
    if (p === "shrink") return "cognitive 12";
    return "cognitive 12";
  };

  return (
    <div
      class="relative mx-auto w-full max-w-md select-none"
      role="img"
      aria-label="Isometric view: a function block grows in complexity, leadline scans it, then the block shrinks after a fix"
    >
      <svg
        viewBox="0 0 320 240"
        class="w-full max-w-full overflow-hidden"
        xmlns="http://www.w3.org/2000/svg"
      >
        <defs>
          <linearGradient id="top" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stop-color="#1a2836" />
            <stop offset="100%" stop-color="#121a24" />
          </linearGradient>
          <linearGradient id="left" x1="0" y1="0" x2="1" y2="0">
            <stop offset="0%" stop-color="#0d1218" />
            <stop offset="100%" stop-color="#151f2a" />
          </linearGradient>
          <linearGradient id="right" x1="0" y1="0" x2="1" y2="0">
            <stop offset="0%" stop-color="#182430" />
            <stop offset="100%" stop-color="#0f161e" />
          </linearGradient>
        </defs>

        {/* Grid plane */}
        <g opacity="0.35" stroke="#1e2936" stroke-width="0.5">
          {Array.from({ length: 9 }, (_, i) => (
            <line x1={40 + i * 28} y1="200" x2={40 + i * 28 - 56} y2="128" />
          ))}
          {Array.from({ length: 7 }, (_, i) => (
            <line x1="40" y1={200 - i * 18} x2="264" y2={200 - i * 18 - 56} />
          ))}
        </g>

        {/* Stack of smaller blocks (unchanged functions) */}
        <IsoBlock x={72} y={168} w={48} h={22} label="fn_a" dim />
        <IsoBlock x={128} y={152} w={44} h={20} label="fn_b" dim />

        {/* Main function block — animated height */}
        <g
          style={{
            transition: reduced ? "none" : "transform 600ms ease-in-out",
          }}
        >
          <IsoBlock
            x={168}
            y={200 - height()}
            w={72}
            h={height()}
            label="processPayment"
            highlight={phase() === "grow" || phase() === "scan"}
            accent={phase() === "scan"}
          />
        </g>

        {/* Scan beam */}
        {phase() === "scan" && (
          <line
            x1="120"
            y1={80 + scanLine() * 100}
            x2="280"
            y2={80 + scanLine() * 100}
            stroke="#3dd68c"
            stroke-width="2"
            stroke-dasharray="6 4"
            opacity="0.85"
          />
        )}

        <text x="168" y="28" fill="#8b9cb0" font-size="10" font-family="monospace">
          leadline changed
        </text>
        <text x="168" y="44" fill="#3dd68c" font-size="11" font-family="monospace">
          {cognitiveLabel()}
        </text>
        {(phase() === "done" || phase() === "shrink") && (
          <text x="168" y="58" fill="#3dd68c" font-size="9" font-family="monospace">
            regressions: []
          </text>
        )}
        {phase() === "grow" && (
          <text x="168" y="58" fill="#f07178" font-size="9" font-family="monospace">
            regressions: 1
          </text>
        )}
      </svg>
      <p class="mt-2 text-center text-[10px] text-muted">
        Pair functions by name · delta per metric · agent-json on stdout
      </p>
    </div>
  );
}

function IsoBlock(props: {
  x: number;
  y: number;
  w: number;
  h: number;
  label: string;
  dim?: boolean;
  highlight?: boolean;
  accent?: boolean;
}) {
  const { x, y, w, h, label, dim, highlight, accent } = props;
  const depth = 14;
  const topColor = dim ? "#141c26" : highlight ? "#1e3a2f" : "#1a2836";
  const leftColor = dim ? "#0a0e14" : "#121a24";
  const rightColor = dim ? "#0f141c" : "#151f2a";
  const stroke = accent ? "#3dd68c" : highlight ? "#f5a623" : "#1e2936";

  return (
    <g>
      <polygon
        points={`${x},${y} ${x + w},${y} ${x + w + depth},${y - depth} ${x + depth},${y - depth}`}
        fill={topColor}
        stroke={stroke}
        stroke-width="0.75"
      />
      <polygon
        points={`${x},${y} ${x + depth},${y - depth} ${x + depth},${y - depth + h} ${x},${y + h}`}
        fill={leftColor}
        stroke={stroke}
        stroke-width="0.75"
      />
      <polygon
        points={`${x + w},${y} ${x + w + depth},${y - depth} ${x + w + depth},${y - depth + h} ${x + w},${y + h}`}
        fill={rightColor}
        stroke={stroke}
        stroke-width="0.75"
      />
      <text
        x={x + w / 2}
        y={y + h / 2 + 4}
        text-anchor="middle"
        fill={dim ? "#5c6d80" : "#e8eef6"}
        font-size="9"
        font-family="monospace"
      >
        {label}
      </text>
    </g>
  );
}
