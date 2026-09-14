# Metric specification: `default`

This specification is project-defined. It does not claim exact Sonar or another analyzer compatibility.

## Function size

Physical LOC is the inclusive function source range. Logical LOC counts declarations, expression statements, control statements, jumps, returns, and throws. Parameter count uses direct grammar parameter nodes. Maximum nesting starts at zero inside the function body.

## Cyclomatic complexity

McCabe basis: each function starts at 1 (one path through straight-line
code). Add 1 per independent branch point. The table below is normative
for `default` and follows SonarQube per-language rules (Java versus
JavaScript, TypeScript, and TSX).

| Construct | Java | JS/TS |
|---|---|---|
| `if` (including each `else if`) | +1 | +1 |
| Loops: `for`, `for-in`/`for-of`, enhanced `for`, `while`, `do-while` | +1 | +1 |
| `catch` | +0 | +1 |
| Non-default switch case (`switch_case`, or `switch_label` starting with `case`) | +1 | +1 |
| `default:` label | +0 | +0 |
| Ternary expression (`?:`) | +1 | +1 |
| Each `&&`, `\|\|` operator occurrence | +1 | +1 |
| `??` | n/a (no such operator) | +0 |
| `throw` | +0 | +1 |
| `->` (lambda header or switch arrow, counted in the enclosing function) | +1 | n/a |
| `switch` header itself | +0 | +0 |
| Function, method, lambda, or arrow definition header | +0 | +0 (base 1 covers Sonar's per-function +1) |
| `else`, `finally` | +0 | +0 |
| `?.`, `??=`, labeled/unlabeled `break`/`continue`, `return` | +0 | +0 |

Nested functions are independent: a lambda or arrow body never adds to
the enclosing function; it is scored as its own function. The only
addition to the enclosing score is +1 per Java `->`
(`src/parser.rs:537,603`, `src/core.rs:287-296`).

### Known SonarQube deltas

The table above matches SonarQube per-language keyword rules. The
remaining deltas are bookkeeping, not scoring:

- `default:` wording: Sonar documents Java as a `case`-only list and
  JS/TS as a `case`-clause list; leadline scores `default:` +0 in every
  language.
- Function-header accounting: Sonar counts +1 per JS/TS function while
  Java methods start from the base; leadline's base 1 covers both, so
  scores agree.
- Java `do-while` is documented by Sonar under the `while` keyword;
  leadline lists it as its own loop row — same score.
- Sonar sums function scores into file/project totals and does not
  show function-level values; leadline reports and gates per function,
  so thresholds are not 1:1.

## Cognitive complexity

Add `1 + current nesting` for `if`, loops, `catch`, `switch`, and ternary expressions. An `else if` adds 1 and continues the original chain. A final `else` adds 1. A labeled `break` or `continue` adds 1. Structural constructs increase nesting for structural descendants.

For each logical expression, the first `&&` or `||` adds 1. A change to the other operator adds 1. Repeated adjacent operators add nothing. Parentheses do not start a new sequence. Recursion is not scored in `default` because syntax alone cannot resolve calls safely.

Lambdas, arrows, and nested functions receive independent scores. Their bodies do not increase the enclosing function score.

## Halstead

Leaf operator tokens and control keywords are operators. Identifiers, literals, `this`, `super`, `true`, `false`, and `null` are operands. Source byte slices define distinct values within one function.

- vocabulary: `n1 + n2`
- length: `N1 + N2`
- volume: `length * log2(vocabulary)`
- difficulty: `(n1 / 2) * (N2 / n2)`
- effort: `difficulty * volume`

Zero denominators produce zero, not NaN.

## Maintainability index

The normalized index is:

```text
max(0, (171 - 5.2 ln(volume) - 0.23 cyclomatic - 16.2 ln(LOC)) * 100 / 171)
```

Volume and LOC use a floor of 1. The result is capped at 100. Comment weighting is not used.

## Coverage and CRAP

Function coverage is the ratio of covered known executable lines to all known executable lines within its inclusive range. Coverage is unavailable when no coverage line overlaps.

CRAP uses cyclomatic complexity `c` and normalized coverage `p`:

```text
c² * (1 - p)³ + c
```

CRAP is unavailable when coverage is unavailable.
