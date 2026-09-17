import React from 'react';
import {
  AbsoluteFill,
  Easing,
  interpolate,
  spring,
  useCurrentFrame,
  useVideoConfig,
} from 'remotion';
import {loadFont} from '@remotion/google-fonts/JetBrainsMono';

loadFont('normal', {weights: ['400', '500', '700']});

// ---------------------------------------------------------------------------
// Leadline promo — 30s @30fps, 1920x1080, dark developer-terminal aesthetic.
// Typeface: JetBrains Mono everywhere (agent-terminal voice).
//
// Scene map (global frames, sharp cuts — no TransitionSeries overlap math):
//   Scene 1   000-110   Problem (agent ships it)  ("THE CODE CHANGED. DID IT GET BETTER?")
//   Scene 2   110-250   Agent calls Leadline      ("ANALYZE THE CHANGE.")
//   Scene 3   250-390   Regression                ("SEE WHAT GOT WORSE.")
//   Scene 4   390-530   Improvement               ("REFACTOR. MEASURE. REPEAT.")
//   Scene 5   530-670   Agent gates itself        ("GATE REGRESSIONS BEFORE THEY LAND.")
//   Scene 6   670-810   Agent loop (MCP/tools)    ("BUILT FOR THE AGENT LOOP.")
//   Scene 7   810-900   Brand                     ("LEADLINE / Measure the change.")
// ---------------------------------------------------------------------------

export const FPS = 30;
export const DURATION = 900;

const S1 = 0;
const S2 = 110;
const S3 = 250;
const S4 = 390;
const S5 = 530;
const S6 = 670;
const S7 = 810;

const BG = '#0a0a0a';
const PANEL = '#0d1117';
const PANEL_EDGE = 'rgba(255,255,255,0.1)';
const TXT = '#ffffff';
const SUB: string = 'rgba(255,255,255,0.6)';
const MUT: string = 'rgba(255,255,255,0.35)';
const GREEN = '#4ade80';
const RED = '#f87171';
const BLUE = '#38bdf8';
const MONO =
  "'JetBrains Mono','SFMono-Regular',Menlo,Consolas,'Liberation Mono',monospace";

// Code syntax palette (restrained GitHub-dark-ish)
const C_PLAIN = '#e6edf3';
const C_KEY = '#ff7b72';
const C_FN = '#d2a8ff';
const C_PROP = '#79c0ff';
const C_PUNCT: string = 'rgba(230,237,243,0.55)';
const C_COM: string = 'rgba(110,118,129,0.9)';

type Tok = {t: string; c?: string};
type CodeLine = Tok[];

// --- helpers ---------------------------------------------------------------

/** Local (scene-relative) frame, clamped at 0 so springs never go negative. */
function useLf(start: number): number {
  return Math.max(0, useCurrentFrame() - start);
}

function fade(lf: number, delay: number, dur = 18) {
  return {
    opacity: interpolate(lf - delay, [0, dur], [0, 1], {
      extrapolateLeft: 'clamp',
      extrapolateRight: 'clamp',
      easing: Easing.out(Easing.quad),
    }),
    dy: interpolate(lf - delay, [0, dur], [26, 0], {
      extrapolateLeft: 'clamp',
      extrapolateRight: 'clamp',
      easing: Easing.out(Easing.quad),
    }),
  };
}

const BlinkCursor: React.FC<{lf: number; color?: string}> = ({
  lf,
  color = GREEN,
}) => (
  <span
    style={{
      display: 'inline-block',
      width: 14,
      height: 30,
      backgroundColor: color,
      verticalAlign: -5,
      marginLeft: 6,
      opacity: lf % 16 < 8 ? 1 : 0,
    }}
  />
);

const TerminalWindow: React.FC<{
  title: string;
  children: React.ReactNode;
  width?: number;
  fontSize?: number;
}> = ({title, children, width = 1120, fontSize = 27}) => (
  <div
    style={{
      width,
      borderRadius: 14,
      backgroundColor: PANEL,
      border: `1px solid ${PANEL_EDGE}`,
      boxShadow: '0 50px 100px rgba(0,0,0,0.5)',
      overflow: 'hidden',
    }}
  >
    <div
      style={{
        height: 52,
        display: 'flex',
        alignItems: 'center',
        padding: '0 22px',
        gap: 10,
        backgroundColor: '#161b22',
        borderBottom: '1px solid rgba(255,255,255,0.08)',
      }}
    >
      <div
        style={{width: 13, height: 13, borderRadius: '50%', backgroundColor: '#ff5f57'}}
      />
      <div
        style={{width: 13, height: 13, borderRadius: '50%', backgroundColor: '#febc2e'}}
      />
      <div
        style={{width: 13, height: 13, borderRadius: '50%', backgroundColor: '#28c840'}}
      />
      <div
        style={{
          flex: 1,
          textAlign: 'center',
          fontFamily: MONO,
          fontSize: 16,
          color: MUT,
        }}
      >
        {title}
      </div>
      <div style={{width: 59}} />
    </div>
    <div
      style={{
        padding: '34px 40px',
        fontFamily: MONO,
        fontSize,
        lineHeight: 1.75,
        color: TXT,
      }}
    >
      {children}
    </div>
  </div>
);

const SceneHeader: React.FC<{
  lf: number;
  index: string;
  title: string;
  sub?: string;
  accent?: string;
}> = ({lf, index, title, sub, accent = TXT}) => {
  const a = fade(lf, 4);
  const b = fade(lf, 12);
  return (
    <>
      <div
        style={{
          position: 'absolute',
          top: 56,
          left: 80,
          fontFamily: MONO,
          fontSize: 20,
          letterSpacing: 4,
          color: MUT,
          opacity: a.opacity,
        }}
      >
        <span style={{color: GREEN}}>{index}</span>
        {'  —  '}
        {title === '' ? '' : eyebrowFor(index)}
      </div>
      <div style={{position: 'absolute', left: 80, bottom: 96}}>
        <div
          style={{
            fontFamily: MONO,
            fontSize: 76,
            fontWeight: 700,
            letterSpacing: -1,
            color: accent,
            opacity: b.opacity,
            transform: `translateY(${b.dy}px)`,
            lineHeight: 1.05,
          }}
        >
          {title}
        </div>
        {sub ? (
          <div
            style={{
              fontFamily: MONO,
              fontSize: 25,
              color: SUB,
              marginTop: 14,
              opacity: b.opacity,
            }}
          >
            {sub}
          </div>
        ) : null}
      </div>
    </>
  );
};

function eyebrowFor(index: string): string {
  switch (index) {
    case '01':
      return 'THE PROBLEM';
    case '02':
      return 'RUN LEADLINE';
    case '03':
      return 'THE REGRESSION';
    case '04':
      return 'THE FIX';
    case '05':
      return 'QUALITY GATE';
    case '06':
      return 'AGENT-NATIVE';
    default:
      return '';
  }
}

const DrawnCheck: React.FC<{
  lf: number;
  start: number;
  color?: string;
  size?: number;
}> = ({lf, start, color = GREEN, size = 26}) => {
  const p = interpolate(lf - start, [0, 14], [0, 1], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
    easing: Easing.out(Easing.quad),
  });
  const len = 34;
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none">
      <path
        d="M4 12.5l5.5 5.5L20 6.5"
        stroke={color}
        strokeWidth={3}
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeDasharray={len}
        strokeDashoffset={len * (1 - p)}
      />
    </svg>
  );
};

function renderTokens(line: CodeLine, keyPrefix: string) {
  return line.map((tok, i) => (
    <span key={`${keyPrefix}-${i}`} style={{color: tok.c ?? C_PLAIN}}>
      {tok.t}
    </span>
  ));
}

// --- Scene 1: problem (0-110) -----------------------------------------------

const AGENT_LINES = [
  {t: 'Modified 12 files', at: 8},
  {t: 'Refactored payment flow', at: 18},
  {t: 'Updated validation', at: 26},
  {t: 'Done ✓ — ship it?', at: 32},
];

const ProblemScene: React.FC = () => {
  const lf = useLf(S1);
  const {fps} = useVideoConfig();
  const frozen = lf >= 72;

  const scrollY = interpolate(lf, [0, 72], [24, -34], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
  });

  const h1 = spring({
    frame: Math.max(0, lf - 72),
    fps,
    config: {damping: 16, stiffness: 140},
  });
  const h1Scale = interpolate(h1, [0, 1], [0.9, 1]);
  const h1Opacity = interpolate(h1, [0, 0.4], [0, 1], {
    extrapolateRight: 'clamp',
  });

  const h2 = spring({
    frame: Math.max(0, lf - 84),
    fps,
    config: {damping: 11, stiffness: 170},
  });
  const h2Scale = interpolate(h2, [0, 1], [1.18, 1]);
  const h2Opacity = interpolate(h2, [0, 0.3], [0, 1], {
    extrapolateRight: 'clamp',
  });
  // Short impact flash on the second headline
  const flash = interpolate(lf - 84, [0, 5], [0.22, 0], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
  });

  return (
    <AbsoluteFill style={{backgroundColor: '#000'}}>
      <AbsoluteFill
        style={{
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          justifyContent: 'center',
        }}
      >
        <div
          style={{
            fontFamily: MONO,
            fontSize: 30,
            lineHeight: 2,
            color: SUB,
            transform: `translateY(${scrollY}px)`,
            textAlign: 'left',
            minHeight: 260,
          }}
        >
          <div
            style={{
              opacity: interpolate(lf - 2, [0, 10], [0, 1], {
                extrapolateLeft: 'clamp',
                extrapolateRight: 'clamp',
              }),
              fontSize: 20,
              letterSpacing: 4,
              color: MUT,
              marginBottom: 6,
            }}
          >
            <span style={{color: GREEN}}>{'● '}</span>
            AUTONOMOUS CODING AGENT — SESSION 042
          </div>
          {AGENT_LINES.map((line) => {
            const vis = interpolate(lf - line.at, [0, 10], [0, 1], {
              extrapolateLeft: 'clamp',
              extrapolateRight: 'clamp',
            });
            const y = interpolate(lf - line.at, [0, 10], [18, 0], {
              extrapolateLeft: 'clamp',
              extrapolateRight: 'clamp',
            });
            return (
              <div
                key={line.t}
                style={{opacity: frozen ? Math.max(vis, 0.55) : vis, transform: `translateY(${y}px)`}}
              >
                <span style={{color: GREEN}}>{'› '}</span>
                {line.t}
              </div>
            );
          })}
          {!frozen ? (
            <div>
              <span style={{color: GREEN}}>{'› '}</span>
              <BlinkCursor lf={lf} />
            </div>
          ) : null}
        </div>

        <div style={{height: 40}} />

        <div
          style={{
            fontFamily: MONO,
            fontSize: 88,
            fontWeight: 700,
            letterSpacing: -2,
            color: TXT,
            opacity: h1Opacity,
            transform: `scale(${h1Scale})`,
          }}
        >
          THE CODE CHANGED.
        </div>
        <div
          style={{
            fontFamily: MONO,
            fontSize: 88,
            fontWeight: 700,
            letterSpacing: -2,
            color: GREEN,
            marginTop: 8,
            opacity: h2Opacity,
            transform: `scale(${h2Scale})`,
          }}
        >
          DID IT GET BETTER?
        </div>
      </AbsoluteFill>
      {flash > 0 ? (
        <AbsoluteFill style={{backgroundColor: '#fff', opacity: flash}} />
      ) : null}
    </AbsoluteFill>
  );
};

// --- Scene 2: analyze (110-250) --------------------------------------------

const CMD2 = 'leadline changed --base origin/main --format agent-json';

const JsonRow: React.FC<{
  lf: number;
  at: number;
  indentKey: string;
  value: string;
  valueColor?: string;
  emphasize?: boolean;
  fps: number;
}> = ({lf, at, indentKey, value, valueColor = '#e3b341', emphasize, fps}) => {
  const vis = interpolate(lf - at, [0, 4], [0, 1], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
  });
  const y = interpolate(lf - at, [0, 4], [10, 0], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
  });
  const pop = emphasize
    ? spring({
        frame: Math.max(0, lf - at),
        fps,
        config: {damping: 10, stiffness: 160},
      })
    : 1;
  const scale = emphasize ? interpolate(pop, [0, 1], [1.12, 1]) : 1;
  return (
    <div
      style={{
        opacity: vis,
        transform: `translateY(${y}px) scale(${scale})`,
        transformOrigin: 'left center',
      }}
    >
      <span style={{color: C_PLAIN}}>{indentKey}</span>
      <span style={{color: C_PUNCT}}>: </span>
      <span style={{color: valueColor, fontWeight: emphasize ? 700 : 400}}>{value}</span>
    </div>
  );
};

const JsonBrace: React.FC<{lf: number; at: number; brace: string}> = ({
  lf,
  at,
  brace,
}) => {
  const vis = interpolate(lf - at, [0, 4], [0, 1], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
  });
  return (
    <div style={{opacity: vis, color: C_PLAIN}}>{brace}</div>
  );
};

const AnalyzeScene: React.FC = () => {
  const lf = useLf(S2);
  const {fps} = useVideoConfig();

  const typed = Math.min(
    CMD2.length,
    Math.max(0, Math.floor((lf - 12) / 1.05)),
  );
  const typingDone = typed >= CMD2.length;
  const jsonStart = 88;

  // Terminal scales 94% -> 100%, then slow push toward the result
  const settle = interpolate(lf, [0, 30], [0.94, 1], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
    easing: Easing.out(Easing.quad),
  });
  const push = interpolate(lf, [70, 150], [1, 1.045], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
  });
  const scale = settle * push;
  const pushY = interpolate(lf, [70, 150], [10, -14], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
  });

  return (
    <AbsoluteFill style={{backgroundColor: BG}}>
      <SceneHeader
        lf={lf}
        index="02"
        title="ANALYZE THE CHANGE."
        sub="Machine-readable evidence for the agent loop."
      />
      <AbsoluteFill
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          paddingBottom: 130,
        }}
      >
        <div style={{transform: `scale(${scale}) translateY(${pushY}px)`}}>
          <TerminalWindow title="agent › terminal — leadline">
            <div>
              <span style={{color: GREEN}}>$ </span>
              <span>{CMD2.slice(0, typed)}</span>
              {!typingDone ? <BlinkCursor lf={lf} /> : null}
            </div>
            <div style={{marginTop: 18}}>
              <JsonBrace lf={lf} at={jsonStart} brace="{" />
              <JsonRow
                lf={lf}
                at={jsonStart + 4}
                fps={fps}
                indentKey='  "changed_functions"'
                value="1,"
              />
              <JsonRow
                lf={lf}
                at={jsonStart + 8}
                fps={fps}
                indentKey='  "regressions"'
                value="1,"
                valueColor={RED}
                emphasize
              />
              <JsonRow
                lf={lf}
                at={jsonStart + 12}
                fps={fps}
                indentKey='  "improvements"'
                value="0"
              />
              <JsonBrace lf={lf} at={jsonStart + 16} brace="}" />
            </div>
          </TerminalWindow>
        </div>
      </AbsoluteFill>
    </AbsoluteFill>
  );
};

// --- Shared code/metrics visuals for scenes 3 + 4 --------------------------

const OLD_CODE: CodeLine[] = [
  [
    {t: 'function ', c: C_KEY},
    {t: 'processPayment', c: C_FN},
    {t: '(order) {', c: C_PUNCT},
  ],
  [
    {t: '  if ', c: C_KEY},
    {t: '(order.valid) {', c: C_PLAIN},
  ],
  [
    {t: '    if ', c: C_KEY},
    {t: '(order.customer) {', c: C_PLAIN},
  ],
  [
    {t: '      if ', c: C_KEY},
    {t: '(order.customer.active) {', c: C_PLAIN},
  ],
  [{t: '        // ...', c: C_COM}],
  [{t: '      }', c: C_PUNCT}],
  [{t: '    }', c: C_PUNCT}],
  [{t: '  }', c: C_PUNCT}],
  [{t: '}', c: C_PUNCT}],
];

const NEW_CODE: CodeLine[] = [
  [
    {t: 'function ', c: C_KEY},
    {t: 'processPayment', c: C_FN},
    {t: '(order) {', c: C_PUNCT},
  ],
  [
    {t: '  validateOrder', c: C_PROP},
    {t: '(order);', c: C_PLAIN},
  ],
  [
    {t: '  validateCustomer', c: C_PROP},
    {t: '(order.customer);', c: C_PLAIN},
  ],
  [
    {t: '  return ', c: C_KEY},
    {t: 'executePayment', c: C_PROP},
    {t: '(order);', c: C_PLAIN},
  ],
  [{t: '}', c: C_PUNCT}],
];

const CodePanel: React.FC<{
  lf: number;
  lines: CodeLine[];
  start: number;
  step?: number;
  fileLabel: string;
}> = ({lf, lines, start, step = 7, fileLabel}) => (
  <div
    style={{
      width: 940,
      borderRadius: 14,
      backgroundColor: PANEL,
      border: `1px solid ${PANEL_EDGE}`,
      boxShadow: '0 50px 100px rgba(0,0,0,0.5)',
      overflow: 'hidden',
    }}
  >
    <div
      style={{
        padding: '14px 28px',
        backgroundColor: '#161b22',
        borderBottom: '1px solid rgba(255,255,255,0.08)',
        fontFamily: MONO,
        fontSize: 18,
        color: MUT,
      }}
    >
      {fileLabel}
    </div>
    <div style={{padding: '28px 36px', fontFamily: MONO, fontSize: 25, lineHeight: 1.8}}>
      {lines.map((line, i) => {
        const vis = interpolate(lf - (start + i * step), [0, 6], [0, 1], {
          extrapolateLeft: 'clamp',
          extrapolateRight: 'clamp',
        });
        const x = interpolate(lf - (start + i * step), [0, 6], [-14, 0], {
          extrapolateLeft: 'clamp',
          extrapolateRight: 'clamp',
        });
        return (
          <div
            key={i}
            style={{
              opacity: vis,
              transform: `translateX(${x}px)`,
              whiteSpace: 'pre',
            }}
          >
            {renderTokens(line, `l${i}`)}
          </div>
        );
      })}
    </div>
  </div>
);

const MetricCard: React.FC<{
  lf: number;
  label: string;
  from: number;
  to: number;
  start: number;
  dur?: number;
  goodWhenDown?: boolean;
  fps: number;
}> = ({lf, label, from, to, start, dur = 40, goodWhenDown, fps}) => {
  const val = Math.round(
    interpolate(lf, [start, start + dur], [from, to], {
      extrapolateLeft: 'clamp',
      extrapolateRight: 'clamp',
      easing: Easing.out(Easing.quad),
    }),
  );
  const done = lf >= start + dur;
  const land = spring({
    frame: Math.max(0, lf - (start + dur)),
    fps,
    config: {damping: 12, stiffness: 170},
  });
  const pop = interpolate(land, [0, 1], [1.22, 1]);
  const up = to > from;
  // Scene 3 (regression): rising values are bad -> red. Scene 4
  // (goodWhenDown): values resolve red -> green once counting finishes.
  const color = goodWhenDown ? (done ? GREEN : RED) : up ? RED : TXT;
  const lineW = interpolate(lf - (start - 6), [0, 10], [0, 56], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
  });
  return (
    <div style={{display: 'flex', alignItems: 'center', gap: 0}}>
      <div
        style={{
          width: 56,
          height: 2,
          backgroundColor: 'rgba(255,255,255,0.18)',
          transform: `scaleX(${lineW / 56})`,
          transformOrigin: 'right center',
        }}
      />
      <div
        style={{
          minWidth: 360,
          borderRadius: 12,
          backgroundColor: PANEL,
          border: `1px solid ${PANEL_EDGE}`,
          padding: '20px 32px',
        }}
      >
        <div style={{fontFamily: MONO, fontSize: 19, letterSpacing: 3, color: MUT}}>
          {label}
        </div>
        <div
          style={{
            display: 'flex',
            alignItems: 'baseline',
            gap: 18,
            marginTop: 8,
            fontFamily: MONO,
          }}
        >
          <span style={{fontSize: 30, color: MUT}}>{from}</span>
          <span style={{fontSize: 26, color: MUT}}>{'→'}</span>
          <span
            style={{
              fontSize: 62,
              fontWeight: 700,
              color,
              transform: `scale(${pop})`,
              transformOrigin: 'left center',
              lineHeight: 1,
            }}
          >
            {val}
          </span>
        </div>
      </div>
    </div>
  );
};

// --- Scene 3: regression (250-390) -----------------------------------------

const RegressionScene: React.FC = () => {
  const lf = useLf(S3);
  const {fps} = useVideoConfig();
  return (
    <AbsoluteFill style={{backgroundColor: BG}}>
      <SceneHeader lf={lf} index="03" title="SEE WHAT GOT WORSE." />
      <AbsoluteFill
        style={{
          display: 'flex',
          flexDirection: 'row',
          alignItems: 'center',
          justifyContent: 'center',
          gap: 40,
          paddingBottom: 120,
        }}
      >
        <CodePanel lf={lf} lines={OLD_CODE} start={15} fileLabel="payment.ts — processPayment" />
        <div style={{display: 'flex', flexDirection: 'column', gap: 22}}>
          <MetricCard lf={lf} fps={fps} label="COGNITIVE" from={12} to={24} start={65} />
          <MetricCard lf={lf} fps={fps} label="CYCLOMATIC" from={8} to={13} start={75} />
          <MetricCard lf={lf} fps={fps} label="NESTING" from={2} to={4} start={85} />
        </div>
      </AbsoluteFill>
    </AbsoluteFill>
  );
};

// --- Scene 4: improvement (390-530) ----------------------------------------

const ImprovementScene: React.FC = () => {
  const lf = useLf(S4);
  const {fps} = useVideoConfig();

  // Old code collapses outward, new code reveals — a morph, not a hard cut
  const oldOpacity = interpolate(lf, [5, 35], [1, 0], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
  });
  const oldY = interpolate(lf, [5, 35], [0, -18], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
  });
  const showNew = lf >= 28;
  const s1 = fade(lf, 100);
  const s2 = fade(lf, 108);

  return (
    <AbsoluteFill style={{backgroundColor: BG}}>
      <SceneHeader
        lf={lf}
        index="04"
        title="REFACTOR. MEASURE. REPEAT."
        accent={GREEN}
      />
      <AbsoluteFill
        style={{
          display: 'flex',
          flexDirection: 'row',
          alignItems: 'center',
          justifyContent: 'center',
          gap: 40,
          paddingBottom: 120,
        }}
      >
        <div style={{position: 'relative', width: 940}}>
          <div style={{opacity: oldOpacity, transform: `translateY(${oldY}px)`}}>
            <CodePanel lf={1000} lines={OLD_CODE} start={0} fileLabel="payment.ts — processPayment" />
          </div>
          {showNew ? (
            <div style={{position: 'absolute', top: 0, left: 0}}>
              <CodePanel
                lf={lf}
                lines={NEW_CODE}
                start={30}
                step={6}
                fileLabel="payment.ts — processPayment"
              />
            </div>
          ) : null}
        </div>
        <div style={{display: 'flex', flexDirection: 'column', gap: 22}}>
          <MetricCard lf={lf} fps={fps} label="COGNITIVE" from={24} to={11} start={48} goodWhenDown />
          <MetricCard lf={lf} fps={fps} label="CYCLOMATIC" from={13} to={7} start={58} goodWhenDown />
          <MetricCard lf={lf} fps={fps} label="NESTING" from={4} to={1} start={68} goodWhenDown />
          <div
            style={{
              display: 'flex',
              flexDirection: 'column',
              gap: 10,
              marginTop: 10,
              fontFamily: MONO,
              fontSize: 26,
              color: GREEN,
            }}
          >
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 14,
                opacity: s1.opacity,
                transform: `translateY(${s1.dy}px)`,
              }}
            >
              <DrawnCheck lf={lf} start={100} />
              <span>1 improvement</span>
            </div>
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 14,
                opacity: s2.opacity,
                transform: `translateY(${s2.dy}px)`,
              }}
            >
              <DrawnCheck lf={lf} start={108} />
              <span>0 regressions</span>
            </div>
          </div>
        </div>
      </AbsoluteFill>
    </AbsoluteFill>
  );
};

// --- Scene 5: quality gate (530-670) ---------------------------------------

const CMD5 = 'leadline check . --cognitive 15 --cyclomatic 10 --max-nesting 4';

const GateScene: React.FC = () => {
  const lf = useLf(S5);

  const typed = Math.min(CMD5.length, Math.max(0, Math.floor((lf - 10) / 0.55)));
  const typingDone = typed >= CMD5.length;

  const failAt = 55;
  const failVis = interpolate(lf - failAt, [0, 6], [0, 1], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
  });
  // Horizontal shake, 2-3px, decaying
  const shake =
    lf >= failAt && lf < 88
      ? Math.sin((lf - failAt) * 1.6) * 3 * (1 - (lf - failAt) / 33)
      : 0;

  // Wipe replaces failure with success
  const wipe = interpolate(lf, [95, 107], [100, 0], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
    easing: Easing.out(Easing.quad),
  });

  const labels = ['LOCAL', 'CI', 'AGENTS'];
  const lineGrow = interpolate(lf, [108, 132], [0, 1], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
    easing: Easing.out(Easing.quad),
  });

  return (
    <AbsoluteFill style={{backgroundColor: BG}}>
      <SceneHeader lf={lf} index="05" title="GATE REGRESSIONS BEFORE THEY LAND." />
      <AbsoluteFill
        style={{
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          justifyContent: 'center',
          paddingBottom: 130,
          gap: 34,
        }}
      >
        <TerminalWindow title="terminal — leadline" width={1240} fontSize={25}>
          <div>
            <span style={{color: GREEN}}>$ </span>
            <span>{CMD5.slice(0, typed)}</span>
            {!typingDone ? <BlinkCursor lf={lf} /> : null}
          </div>
          <div style={{position: 'relative', marginTop: 20, minHeight: 120}}>
            <div style={{opacity: failVis, transform: `translateX(${shake}px)`}}>
              <div style={{color: RED, fontWeight: 700}}>✕ payment.ts::processPayment</div>
              <div style={{color: SUB}}>
                {'  cognitive '}
                <span style={{color: RED, fontWeight: 700}}>24 &gt; 15</span>
              </div>
            </div>
            {lf >= 95 ? (
              <div
                style={{
                  position: 'absolute',
                  top: 0,
                  left: 0,
                  right: 0,
                  bottom: 0,
                  backgroundColor: PANEL,
                  clipPath: `inset(0 ${wipe}% 0 0)`,
                }}
              >
                <div style={{display: 'flex', alignItems: 'center', gap: 14}}>
                  <DrawnCheck lf={lf} start={97} size={30} />
                  <span style={{color: GREEN, fontWeight: 700}}>Quality gate passed</span>
                </div>
              </div>
            ) : null}
          </div>
        </TerminalWindow>

        <div style={{display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 16}}>
          <div
            style={{
              width: 560,
              height: 2,
              backgroundColor: 'rgba(255,255,255,0.14)',
              transform: `scaleX(${lineGrow})`,
            }}
          />
          <div style={{display: 'flex', gap: 28}}>
            {labels.map((label, i) => {
              const f = fade(lf, 110 + i * 5, 8);
              const isAgents = label === 'AGENTS';
              return (
                <div
                  key={label}
                  style={{
                    opacity: f.opacity,
                    transform: `translateY(${f.dy}px)`,
                    fontFamily: MONO,
                    fontSize: 22,
                    letterSpacing: 3,
                    color: isAgents ? GREEN : TXT,
                    border: isAgents ? `1px solid ${GREEN}` : `1px solid ${PANEL_EDGE}`,
                    backgroundColor: PANEL,
                    boxShadow: isAgents ? '0 0 32px rgba(74,222,128,0.22)' : 'none',
                    borderRadius: 999,
                    padding: '12px 30px',
                  }}
                >
                  <span style={{color: GREEN}}>{'● '}</span>
                  {label}
                </div>
              );
            })}
          </div>
        </div>
      </AbsoluteFill>
    </AbsoluteFill>
  );
};

// --- Scene 6: agent loop (670-810) ------------------------------------------
// The agent calls Leadline as MCP tools, reads agent-json evidence, iterates:
// iteration 1 finds the regression, iteration 2 verifies the fix.

const NODES = ['AI AGENT', 'EDIT', 'LEADLINE', 'EVIDENCE'];
const NODE_Y = [310, 452, 594, 736];

const TOOL_CALLS = [
  {tool: 'analyze', args: 'payment.ts'},
  {tool: 'check', args: '--cognitive 15'},
];

const SURFACES = ['MCP SERVER', 'AGENT-JSON', 'AGENT SKILL'];

const AgentLoopScene: React.FC = () => {
  const lf = useLf(S6);
  const {fps} = useVideoConfig();

  const rotation = interpolate(lf, [0, 140], [0, 360], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
  });

  const top = NODE_Y[0];
  const bottom = NODE_Y[NODES.length - 1];
  const span = bottom - top;
  const packets = [0, 1].map((k) => top + ((lf * 4.6 + k * 213) % span));
  // Hide the travelling evidence label while it passes behind a node
  const chipOpacity = NODE_Y.some((y) => Math.abs(packets[0] - y) < 58) ? 0 : 1;

  const pass1 = fade(lf, 64, 10);
  const pass1Out = interpolate(lf, [94, 102], [1, 0], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
  });
  const pass2 = fade(lf, 100, 10);
  const toolsCap = fade(lf, 40, 10);
  const surfCap = fade(lf, 90, 10);

  return (
    <AbsoluteFill style={{backgroundColor: BG}}>
      <SceneHeader lf={lf} index="06" title="BUILT FOR THE AGENT LOOP." />
      <AbsoluteFill
        style={{display: 'flex', alignItems: 'center', justifyContent: 'center', paddingBottom: 60}}
      >
        {/* rotating loop ring */}
        <div
          style={{
            position: 'absolute',
            width: 700,
            height: 700,
            left: 960 - 350,
            top: 523 - 350,
            transform: `rotate(${rotation}deg)`,
            opacity: 0.35,
          }}
        >
          <div
            style={{
              width: '100%',
              height: '100%',
              borderRadius: '50%',
              border: `2px dashed ${BLUE}`,
            }}
          />
          <div
            style={{
              position: 'absolute',
              top: -9,
              left: '50%',
              width: 0,
              height: 0,
              borderLeft: '9px solid transparent',
              borderRight: '9px solid transparent',
              borderBottom: `14px solid ${BLUE}`,
            }}
          />
        </div>

        {/* connectors */}
        {NODE_Y.slice(0, -1).map((y, i) => {
          const grow = interpolate(lf - (18 + i * 8), [0, 10], [0, 1], {
            extrapolateLeft: 'clamp',
            extrapolateRight: 'clamp',
          });
          return (
            <div
              key={i}
              style={{
                position: 'absolute',
                left: 959,
                top: y + 34,
                width: 2,
                height: NODE_Y[i + 1] - y - 68,
                backgroundColor: 'rgba(255,255,255,0.22)',
                transform: `scaleY(${grow})`,
                transformOrigin: 'top center',
              }}
            />
          );
        })}

        {/* travelling data packets */}
        {packets.map((y, i) => (
          <div
            key={i}
            style={{
              position: 'absolute',
              left: 953,
              top: y,
              width: 14,
              height: 14,
              borderRadius: '50%',
              backgroundColor: GREEN,
              boxShadow: '0 0 18px rgba(74,222,128,0.9)',
            }}
          />
        ))}
        <div
          style={{
            position: 'absolute',
            left: 986,
            top: packets[0] - 16,
            fontFamily: MONO,
            fontSize: 18,
            color: GREEN,
            backgroundColor: PANEL,
            border: `1px solid ${PANEL_EDGE}`,
            borderRadius: 8,
            padding: '6px 12px',
            opacity: chipOpacity,
          }}
        >
          cog 24→11
        </div>

        {/* nodes */}
        {NODES.map((node, i) => {
          const p = spring({
            frame: Math.max(0, lf - (10 + i * 8)),
            fps,
            config: {damping: 15, stiffness: 120},
          });
          const scale = interpolate(p, [0, 1], [0.7, 1]);
          const opacity = interpolate(p, [0, 0.4], [0, 1], {
            extrapolateRight: 'clamp',
          });
          const leadline = node === 'LEADLINE';
          return (
            <div
              key={node}
              style={{
                position: 'absolute',
                left: 960 - 190,
                top: NODE_Y[i] - 34,
                width: 380,
                height: 68,
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                fontFamily: MONO,
                fontSize: 27,
                fontWeight: 700,
                letterSpacing: 3,
                color: leadline ? GREEN : TXT,
                backgroundColor: PANEL,
                border: leadline ? `2px solid ${GREEN}` : `1px solid ${PANEL_EDGE}`,
                borderRadius: 14,
                boxShadow: leadline
                  ? '0 0 44px rgba(74,222,128,0.25)'
                  : '0 18px 40px rgba(0,0,0,0.45)',
                opacity,
                transform: `scale(${scale})`,
              }}
            >
              {node}
            </div>
          );
        })}

        {/* MCP tool calls the agent makes */}
        <div
          style={{
            position: 'absolute',
            left: 1190,
            top: 478,
            fontFamily: MONO,
            fontSize: 19,
            letterSpacing: 4,
            color: MUT,
            opacity: toolsCap.opacity,
          }}
        >
          THE AGENT CALLS
        </div>
        {TOOL_CALLS.map((call, i) => {
          const f = fade(lf, 46 + i * 12, 10);
          return (
            <div
              key={call.tool}
              style={{
                position: 'absolute',
                left: 1190,
                top: 522 + i * 72,
                width: 510,
                opacity: f.opacity,
                transform: `translateY(${f.dy}px)`,
                fontFamily: MONO,
                fontSize: 23,
                backgroundColor: PANEL,
                border: `1px solid ${PANEL_EDGE}`,
                borderRadius: 10,
                padding: '14px 22px',
              }}
            >
              <span style={{color: GREEN}}>mcp › </span>
              <span style={{color: TXT}}>{call.tool}</span>
              <span style={{color: MUT}}> {call.args}</span>
            </div>
          );
        })}

        {/* what the agent integrates */}
        <div
          style={{
            position: 'absolute',
            left: 120,
            top: 478,
            fontFamily: MONO,
            fontSize: 19,
            letterSpacing: 4,
            color: MUT,
            opacity: surfCap.opacity,
          }}
        >
          AGENT SURFACES
        </div>
        {SURFACES.map((surface, i) => {
          const f = fade(lf, 96 + i * 6, 8);
          return (
            <div
              key={surface}
              style={{
                position: 'absolute',
                left: 120,
                top: 522 + i * 72,
                width: 340,
                opacity: f.opacity,
                transform: `translateY(${f.dy}px)`,
                fontFamily: MONO,
                fontSize: 22,
                letterSpacing: 2,
                color: TXT,
                backgroundColor: PANEL,
                border: `1px solid ${PANEL_EDGE}`,
                borderRadius: 999,
                padding: '13px 26px',
              }}
            >
              <span style={{color: GREEN}}>{'● '}</span>
              {surface}
            </div>
          );
        })}

        {/* the loop iterates: red finding, then green verification */}
        <div
          style={{
            position: 'absolute',
            top: 806,
            width: '100%',
            textAlign: 'center',
            fontFamily: MONO,
            fontSize: 26,
            color: RED,
            opacity: pass1.opacity * pass1Out,
            transform: `translateY(${pass1.dy}px)`,
          }}
        >
          iteration 1<span style={{color: MUT}}>{' → '}</span>regressions: 1
          <span style={{color: MUT}}>{' · agent refactors…'}</span>
        </div>
        <div
          style={{
            position: 'absolute',
            top: 806,
            width: '100%',
            textAlign: 'center',
            fontFamily: MONO,
            fontSize: 26,
            fontWeight: 700,
            color: GREEN,
            opacity: pass2.opacity,
            transform: `translateY(${pass2.dy}px)`,
          }}
        >
          iteration 2<span style={{color: MUT}}>{' → '}</span>regressions: 0 ✓
        </div>
      </AbsoluteFill>
    </AbsoluteFill>
  );
};

// --- Scene 7: brand (810-900) ----------------------------------------------

const LogoMark: React.FC<{size: number}> = ({size}) => (
  <svg width={size} height={size} viewBox="0 0 120 120">
    <rect width="120" height="120" rx="26" fill="#0e1626" />
    <line x1="60" y1="14" x2="60" y2="72" stroke="#e8eef6" strokeWidth="4" strokeLinecap="round" />
    <line x1="46" y1="28" x2="60" y2="28" stroke="#64748b" strokeWidth="4" strokeLinecap="round" />
    <line x1="46" y1="42" x2="60" y2="42" stroke="#64748b" strokeWidth="4" strokeLinecap="round" />
    <line x1="46" y1="56" x2="60" y2="56" stroke="#64748b" strokeWidth="4" strokeLinecap="round" />
    <path d="M60 72 l10 12 -10 12 -10 -12 Z" fill={BLUE} />
    <path
      d="M22 106 q9 -8 19 0 t19 0 t19 0 t19 0"
      fill="none"
      stroke={BLUE}
      strokeWidth="4"
      strokeLinecap="round"
    />
  </svg>
);

const BrandScene: React.FC = () => {
  const lf = useLf(S7);

  const dropY = interpolate(lf, [0, 28], [-320, 0], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
    easing: Easing.out(Easing.cubic),
  });
  const logoOpacity = interpolate(lf, [0, 8], [0, 1], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
  });
  const ls = interpolate(lf, [30, 60], [20, 5], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
    easing: Easing.out(Easing.quad),
  });
  const word = fade(lf, 30, 22);
  const tag = fade(lf, 52, 18);
  const cmd = fade(lf, 64, 16);

  return (
    <AbsoluteFill style={{backgroundColor: BG}}>
      <AbsoluteFill
        style={{
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          justifyContent: 'center',
        }}
      >
        <div style={{position: 'relative', width: 200, height: 200}}>
          {/* ripple rings */}
          {[0, 1, 2].map((i) => {
            const t = interpolate(lf - (28 + i * 6), [0, 26], [0, 1], {
              extrapolateLeft: 'clamp',
              extrapolateRight: 'clamp',
              easing: Easing.out(Easing.quad),
            });
            return (
              <div
                key={i}
                style={{
                  position: 'absolute',
                  left: '50%',
                  top: '50%',
                  width: 190,
                  height: 190,
                  borderRadius: '50%',
                  border: `2px solid ${BLUE}`,
                  opacity: (1 - t) * 0.55,
                  transform: `translate(-50%,-50%) scale(${0.55 + t * 1.15})`,
                }}
              />
            );
          })}
          <div
            style={{
              position: 'absolute',
              left: 15,
              top: 0,
              opacity: logoOpacity,
              transform: `translateY(${dropY}px)`,
            }}
          >
            <LogoMark size={170} />
          </div>
        </div>

        <div style={{height: 30}} />

        <div
          style={{
            fontFamily: MONO,
            fontSize: 118,
            fontWeight: 700,
            color: TXT,
            letterSpacing: ls,
            opacity: word.opacity,
            transform: `translateY(${word.dy}px)`,
            lineHeight: 1,
          }}
        >
          LEADLINE
        </div>
        <div
          style={{
            fontFamily: MONO,
            fontSize: 32,
            color: SUB,
            marginTop: 18,
            opacity: tag.opacity,
          }}
        >
          Measure the change.
        </div>
        <div
          style={{
            marginTop: 34,
            fontFamily: MONO,
            fontSize: 26,
            color: TXT,
            backgroundColor: PANEL,
            border: `1px solid ${PANEL_EDGE}`,
            borderRadius: 10,
            padding: '16px 32px',
            opacity: cmd.opacity,
          }}
        >
          <span style={{color: GREEN}}>$ </span>leadline analyze .
          <BlinkCursor lf={lf} />
        </div>
      </AbsoluteFill>
    </AbsoluteFill>
  );
};

// --- main ------------------------------------------------------------------

export const LeadlinePromo: React.FC = () => {
  const frame = useCurrentFrame();
  const progress = frame / DURATION;
  return (
    <AbsoluteFill style={{backgroundColor: BG}}>
      {frame < S2 ? (
        <ProblemScene />
      ) : frame < S3 ? (
        <AnalyzeScene />
      ) : frame < S4 ? (
        <RegressionScene />
      ) : frame < S5 ? (
        <ImprovementScene />
      ) : frame < S6 ? (
        <GateScene />
      ) : frame < S7 ? (
        <AgentLoopScene />
      ) : (
        <BrandScene />
      )}
      {/* global progress hairline */}
      <div
        style={{
          position: 'absolute',
          bottom: 0,
          left: 0,
          height: 4,
          width: 1920 * progress,
          backgroundColor: GREEN,
        }}
      />
    </AbsoluteFill>
  );
};
