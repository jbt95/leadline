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

Only real operator tokens count: `&&`/`||` written inside string,
template, or JSX text are text, not branches, and operators inside a
nested function add to that function's score, never the enclosing
function's.

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

### Go counting policy

Go follows the same table with these adjustments: `default:` arms count
as Case (+1 cyclomatic, +0 cognitive), unlike Java/JS/TS where `default:`
scores +0. `go`/`defer` statements are not decisions (they count toward
logical LOC only). Method receivers are excluded from the parameter
count. Go has no `?`-style error returns; explicit `if err != nil`
checks count as ordinary `if` decisions.

### Rust counting policy

Rust follows the shared cyclomatic and cognitive tables with these
adjustments. `if`/`if let`, `while`/`while let`, `loop`, `for`, `else if`,
and `else` behave exactly as the shared rule states, and `&&`/`||` count
exactly as in every other language, including the `&&` separators between
the `let` conditions of an `if let` chain. `match` follows the `switch`
row: the header is +0 cyclomatic and +1 + nesting cognitive, and each arm
is +1 cyclomatic as a Case and +0 cognitive. Rust has no ternary, `catch`,
or `throw`, so those rows never apply. The `?` try-operator adds 1
cyclomatic and 0 cognitive and raises no nesting: it is an implicit early
return. A labeled `break`/`continue` adds 1 cognitive and direct
self-recursion adds 1, as in every other language. `panic!`,
`unreachable!`, `unwrap()`, and `expect()` are calls, not decisions, and
add nothing. A `self` parameter (`&self`, `&mut self`, `self`,
`self: Box<Self>`) is excluded from the parameter count, the same way a Go
method receiver is not counted. Macro bodies (`println!`, `assert!`,
`json!`, `vec!`) are token trees, not expressions, so decisions, loops,
and `match` inside them are not scored and macro-heavy files under-report;
macro token leaves count toward Halstead only where their text is a
recognized operator or operand. The legacy `try!(...)` macro is a parse
error for this grammar (`try` is a reserved keyword), so those call sites
are reported as parse errors and score nothing; the modern `?` operator
scores as above. Rust files are therefore not score-comparable with
Go/Java/JS/TS files.

### C counting policy

C follows the shared cyclomatic and cognitive rules for conditionals,
loops, switches, ternaries, and `&&`/`||`. The C grammar emits both
`case` and `default` as `case_statement`, so each arm adds 1 cyclomatic
and 0 cognitive. C has no `catch` or `throw` constructs. A
`goto_statement` and its `labeled_statement` each add 1 cognitive and 0
cyclomatic. Direct self-recursion adds 1 cognitive.

Preprocessor conditionals (`#if`, `#ifdef`, `#elif`, and `#else`) count
toward logical LOC but are not decisions. A lone `void` parameter means
zero parameters, and `...` is not a named parameter. Function-like macro
bodies are raw `preproc_arg` text, not expressions, so decisions, loops,
and switches inside them are not scored. Macro-heavy C files can
therefore under-report complexity, like Rust files with control flow
inside macro token trees.
### C++ counting policy

C++ discovers only `function_definition` and `lambda_expression` nodes.
Declarations and prototypes without a body are not functions. Lambdas are
scored independently and use an `auto` initializer or assignment target as
their name when one exists. Inline class or struct definitions and qualified
definitions such as `Type::method` are methods.

C++ follows the shared control-flow rules with these adjustments.
Range-based `for` statements are loops. Every `case_statement`, including a
`default:` label, is a Case and adds 1 cyclomatic. `catch` and `throw` follow
Java semantics: `catch` adds no cyclomatic point, and `throw` adds no
cyclomatic or cognitive point. A `goto` statement and a label each add 1
cognitive point without changing cyclomatic complexity. Preprocessor
conditionals (`#if`, `#ifdef`, `#elif`, and `#else`) are logical lines only;
they are not decisions.

Parameter count includes `parameter_declaration` and
`optional_parameter_declaration` nodes, excludes variadic parameters, and
treats a lone unnamed `void` parameter as zero parameters. Direct calls by a
function's bare or qualified name count as recursion. A call through `this`
also counts, but a same-named call through another object does not.

Function-like macro bodies are raw `preproc_arg` text, not syntax trees.
Decisions and loops inside them are therefore not scored, so macro-heavy
files under-report control flow. Their token text contributes to Halstead
only when the grammar exposes a recognized leaf.

### Python counting policy

Python discovers `function_definition` (`def`, including `async def`) and
`lambda` nodes; a `def` inside a class body is a method, and `.pyi` stubs
are not analyzed at all, because they declare signatures with no bodies.

The shared tables apply with these Python-specific rules:

- `elif_clause` is not an `if_statement`, so it is scored as an else-if:
  each branch of an `if`/`elif` chain adds 1 cyclomatic and 1 cognitive point
  and never raises nesting. The trailing `else_clause` adds 1 cognitive
  point. A chain therefore reads exactly like JavaScript's `if`/`else if`/
  `else`.
- Comprehension clauses score like the explicit loop they replace: each
  `for_in_clause` is a Loop and each `if_clause` is a conditional.
- `except_clause` is a Catch and adds 1 cyclomatic point; Python is not
  Java. `finally_clause` is not a decision. The `else` on `for`, `while`,
  and `try` adds 1 cognitive point, because it is another path a reader must
  follow. This is the one difference from the `for`/`while` rows of the
  other languages, which have no `else` clause to score: no decision tool
  run for this batch reported a usable signal for it, so it was decided
  here. The second such decision is that `assert` scores nothing, matching
  the Rust `panic!` row, because the optimizer strips it.
- `raise_statement` follows JavaScript's `throw`: 1 cyclomatic point and no
  cognitive point.
- `match_statement` is a Switch and each `case_clause` is a Case.
- `with_statement`, `not_operator`, and `global`/`nonlocal`/`pass`/`del`
  statements are not decisions. `and` and `or` are logical operators, scored
  like `&&` and `||`; a logical line is any effect statement, including
  `with`, `raise`, `assert`, `match`/`case`, `del`, `pass`, and the two
  comprehension clauses.
- `self` is an ordinary explicit parameter and is counted. Parameter count
  counts the six carrier node kinds (`identifier`, `typed_parameter`,
  `default_parameter`, `typed_default_parameter`, `list_splat_pattern`,
  `dictionary_splat_pattern`), so `*`/`/` separators do not count and
  `*args` counts once.
- A `lambda` bound by an assignment takes the assignment target as its name.
  A call is recursive when its callee is the bare name or a `self.<name>`
  attribute; an absolute-import-style call such as `os.path.join` is not.
- Operator keywords (`and`, `or`, `not`, `is`, `lambda`, `with`, `as`,
  `assert`, `raise`, `try`, `except`, `finally`, `elif`, `def`, `import`,
  `from`, `del`, `pass`, `global`, `nonlocal`, `async`) count as Halstead
  operators. `match` is already an operator in the shared table. An
  `integer`, `float`, or `none` literal and the `string_content` leaf of a
  string literal count as operands.

## Cognitive complexity

Add `1 + current nesting` for `if`, loops, `catch`, `switch`, and ternary expressions. An `else if` adds 1 and continues the original chain. A final `else` adds 1. A labeled `break` or `continue` adds 1. Structural constructs increase nesting for structural descendants.

For each logical expression, the first `&&` or `||` adds 1. A change to the other operator adds 1. Repeated adjacent operators add nothing. Parentheses do not start a new sequence.

A directly self-recursive function adds 1 (a `recursion` contribution on the function's first line). Mutual/indirect cycles are not scored: syntax alone cannot resolve them.

Lambdas, arrows, and nested functions receive independent scores. Their bodies do not increase the enclosing function score.

### Known SonarQube deltas (cognitive)

The recursion rule above matches SonarQube for direct self-recursion. The
remaining deltas are intentional:

- Indirect (mutual) recursion is not scored: detecting call cycles needs
  whole-program analysis, while each function is scored from its own syntax
  alone.
- `else` and `else if` bodies add no nesting depth: the shared `nesting`
  value also drives the leadline-only `max_nesting` metric, so raising it
  would change both scores at once.
- Nesting never passes through lambdas, arrows, or nested functions: each
  function is scored independently (independent-functions architecture,
  `docs/architecture.md:31`).

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

Function coverage is the ratio of covered known executable lines to all known executable lines within its inclusive range. When branch records are present the ratio mixes line and branch data Sonar-style: (covered lines + covered branches) / (known lines + known branches); reports without branch records (`BRDA`/`mb`+`cb`) keep exact line-only ratios. Coverage is unavailable when no known line or branch record overlaps.

CRAP uses cyclomatic complexity `c` and normalized coverage `p` (the mixed line-and-branch ratio):

```text
c² * (1 - p)³ + c
```

CRAP is unavailable when coverage is unavailable.

## Duplication

Duplication detection uses normalized token sequences (profile `tokens`).
Comments are dropped; identifiers collapse to `<id>`, string and template
text to `<str>`, and every numeric literal syntax — decimal, hex, octal,
binary, and floating-point, including Java `0x1F`, `017`, `0b1010`, and
`0x1.8p3` — to `<num>`. Keywords and punctuation stay exact, and each
language is a separate partition, so a Java clone never matches a
TypeScript clone.

A clone group is a repeated sequence of at least `min_tokens` normalized
tokens (default `100`) whose every occurrence spans at least `min_lines`
lines (default `10`); both are configurable under `[duplication]`.
Detection is bounded (10,000,000 tokens and 10,000,000 exact comparisons
per run); a report that hits a ceiling is marked `complete: false` rather
than dropping candidates silently. `duplication --base REV` compares
occurrence groups between two states, with group statuses
`new`/`existing`/`resolved` and added/removed occurrence counts.
