# Metric specification: `default-v1`

This specification is project-defined. It does not claim exact Sonar or another analyzer compatibility.

## Function size

Physical LOC is the inclusive function source range. Logical LOC counts declarations, expression statements, control statements, jumps, returns, and throws. Parameter count uses direct grammar parameter nodes. Maximum nesting starts at zero inside the function body.

## Cyclomatic complexity

Every function starts at 1. Add 1 for each `if`, loop, `catch`, non-default switch case, ternary expression, `&&`, `||`, and JavaScript or TypeScript `??`. `else`, `finally`, default cases, functions, and lambdas do not add points. Nested functions are independent.

## Cognitive complexity

Add `1 + current nesting` for `if`, loops, `catch`, `switch`, and ternary expressions. An `else if` adds 1 and continues the original chain. A final `else` adds 1. A labeled `break` or `continue` adds 1. Structural constructs increase nesting for structural descendants.

For each logical expression, the first `&&`, `||`, or `??` adds 1. A change to another operator adds 1. Repeated adjacent operators add nothing. Parentheses do not start a new sequence. Recursion is not scored in `default-v1` because syntax alone cannot resolve calls safely.

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
