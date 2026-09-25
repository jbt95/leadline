# Dependencies and impact

Static file-level dependency intelligence for Go, JavaScript, TypeScript, Java, Rust, C, C++, Python, and Zig.
Go imports are module-qualified paths, so `GoImport` references are `Ignored` and never produce edges.
Python absolute imports are module-qualified paths on `sys.path`, so
`PythonAbsoluteImport` references are recorded as `unresolved` with reason
`unsupported` and never produce edges.
Zig graph support is local-only: string `@import` arguments that name exact
relative `.zig` files resolve against the importing file. Bare package names
are ignored; package graphs and Zig builds are not executed.
The analyzer parses code with Tree-sitter and never executes it: no build
runtime, no network, no project scripts.

## Commands

```console
leadline dependencies [PATH] [--json] [--format agent-json]
leadline impact TARGET [--path ROOT] [--top N] [--json] [--format agent-json]
```

- `dependencies`: graph over the files discovery finds under `PATH`
  (default `.`). Honors `leadline.toml` `[analysis] exclude` like `analyze`.
- `impact TARGET`: transitive dependents of one file. `TARGET` must be an
  existing file under the scope (`--path ROOT`, default `.`); a path outside
  the scope, a missing file, or a file that exists but is not in the graph
  (for example a `.txt` file) exits `2`. `--top N` (default 20, `N >= 1`)
  caps the shown `dependents` list; `blast_radius` still counts every
  dependent and `truncated` signals the cap.
- `--json` and `--format agent-json` are exclusive. `--format` accepts only
  `agent-json`. `leadline dependencies --help` and `leadline impact --help`
  print the global usage text with the command line.

```console
leadline dependencies src --json
leadline dependencies . --format agent-json
leadline impact src/payment.ts
leadline impact src/payment.ts --path . --top 50 --json
leadline impact src/payment.ts --format agent-json
```

## Edge direction

Edges point from importer to imported file: `source` imports `target`.
`impact` follows reverse edges, so the dependents of `target` are the files
that (transitively) import it.

## Supported import forms

JavaScript / TypeScript (`.ts`, `.tsx`, `.js`, `.jsx`, `.mts`, `.cts`,
`.mjs`, `.cjs`):

- `import ... from "./x"`, `import "./x"` (static import) → `kind: "import"`.
- `export ... from "./x"`, `export * from "./x"` (re-export) → `kind: "import"`.
- `require("./x")` with a single string argument → `kind: "call"`.
- `import("./x")` with a single string argument → `kind: "call"`.
- Only relative specifiers resolve: `./x`, `../x` (and exactly `.` / `..`).
  Bare package imports (`"third-party"`, `"package-name"`) and dot-prefixed
  bare names (`'.package'`, `'..package'`) are ignored, not reported.
- Resolution is relative to the importing file's directory: exact path match
  first, then extensionless `./x` tries `x.{ts,tsx,js,jsx,mts,cts,mjs,cjs}`
  and `x/index.{ts,tsx,js,jsx,mts,cts,mjs,cjs}`, then `./x.js` / `./x.mjs` /
  `./x.cjs` also try the TypeScript counterparts `x.{ts,tsx,mts,cts}`.
  Duplicate imports of the same target collapse to one edge; `import` wins
  over `call` when both name the same pair.
- String escape sequences are decoded before resolution, so
  `import "./\uD83D\uDE00"` resolves to the emoji-named file.
- Unresolvable relatives appear in `unresolved` with a reason:
  `not_found` (no candidate), `ambiguous` (several candidates, for example
  both `x.ts` and `x.tsx` exist), `outside_scope` (`..` escapes the root),
  `unsupported` (non-source extension such as `.css`, or a literal in
  dependency position that cannot be decoded). Excluded files behave like
  missing files: the edge is `not_found` in `unresolved`.
- Dynamic expressions with a non-string argument (`import(variable)`,
  template literals, concatenation) are not references at all: they produce
  no edge and no `unresolved` row.

Java (`.java`):

- `import pkg.Type;` resolves when exactly one discovered file declares that
  qualified top-level type (class, interface, enum, record, or annotation
  type, qualified by its `package` declaration). Several declarers →
  `ambiguous`; none → `not_found`.
- `import static pkg.Type.member...;` strips trailing members until a known
  type matches (`import static util.Tools.run` → `util.Tools`;
  `import static util.Outer.Inner.member` → `util.Outer`). The `static`
  keyword is matched exactly: `import staticpkg.Outer.Missing;` is an
  ordinary (non-static) import and resolves to `not_found`, not to
  `staticpkg.Outer`.
- `import pkg.*;` is always `unresolved` with reason `unsupported`.
- Same-package uses without an import statement produce no edge. Only
  explicit imports are evidence.

C (`.c`; `.h` headers are C++, see below):

- `#include "path/to/file.c"` resolves the exact path relative to the
  including file's directory → `kind: "import"`. Includes carry their own
  extension, so the resolver does not probe alternatives.
- A quoted include with no discovered exact target is `unresolved` with
  reason `not_found`.
- `#include <system.h>` and macro-built paths such as `#include HEADER`
  are ignored. The system search path and macro expansion environment are
  unknown, so the analyzer does not guess.

Rust (`.rs`):

- `mod foo;` (a module declaration without a body) resolves inside the
  declaring file's module directory to `foo.rs` or `foo/mod.rs` →
  `kind: "import"`. A `mod.rs` file or a crate root (`lib.rs`, `main.rs`,
  `build.rs`, a `bin`/`benches`/`examples`/`tests` target) owns its own
  directory, so `src/parser/mod.rs` reaches `src/parser/rust.rs`; any other
  module file owns the directory named after it, so `src/telemetry.rs`
  reaches `src/telemetry/counters.rs`. Exactly one candidate found resolves;
  both candidates present is `ambiguous`; neither present is `not_found`.
- `mod foo { ... }` (inline module) is not a file reference and produces no
  edge.
- `use ...` paths (`use crate::a::b;`, `use super::x;`, `use self::y;`,
  `use serde::Deserialize;`) and `extern crate` declarations are
  module-qualified, never file-relative: they are ignored and never produce
  edges or `unresolved` rows, the same treatment Go imports get.

C++ (`.h`, `.cpp`, `.cc`, `.cxx`, `.hpp`, `.hh`, `.hxx`):

- Headers belong to C++ because the C++ grammar parses C headers while the
  C grammar fails on C++ headers, so every `.h` file is analyzed here.
- `#include "path/file.hpp"` resolves the exact path relative to the
  including file's directory and produces `kind: "import"`. Includes carry
  their own extension, so resolution does not probe other extensions or
  directory index names.
- A missing quoted include is `unresolved` with reason `not_found`. An
  include whose `..` components leave the analysis root is also
  `not_found`.
- System includes such as `#include <vector>` and macro-built includes such
  as `#include HEADER_FILE` are ignored. The analyzer does not know compiler
  include search paths or macro expansion state, so it does not guess.

Python (`.py`):

- Relative imports — `from . import x`, `from .mod import y`,
  `from ..pkg.sub import z` — resolve to a file relative to the declaring
  file's own directory → `kind: "import"`. The level climbs directories:
  one dot (`.`) stays in the declaring file's own directory, two dots (`..`)
  climb one directory, three dots climb two. Candidates are `<base>.py` and
  `<base>/__init__.py`, so `from ..pkg.sub import z` in `app/svc/api.py`
  names `app/pkg/sub.py` or `app/pkg/sub/__init__.py`.
- A bare `from . import a, b` imports submodules of the current package, so
  it produces one reference per imported name: `a` resolves to `a.py` or
  `a/__init__.py` beside the declaring file, and `b` likewise.
- Relative outcomes: several candidates → `ambiguous` (both `mod.py` and
  `mod/__init__.py` exist); no candidate → `not_found`; a level that climbs
  above the analysis root → `outside_scope`.
- Absolute dotted imports — `import a.b`, `import a.b as c`,
  `from a.b import d` — name a module on `sys.path`, which a parser cannot
  see. They are recorded as `unresolved` with reason `unsupported` (one row
  per module path named), never resolved and never an edge, and the `unused`
  report is `complete: false` with `reason: "unresolved_references"`
  whenever a file imports absolutely.
- `import_statement` and `import_from_statement` nested inside a function,
  class, or `if` block are references too: the whole file is walked, not
  just its top level.

Zig (`.zig`):

- A string argument to `@import` resolves only when it names one exact
  discovered `.zig` file relative to the importing file. A missing local
  target is `unresolved` with reason `not_found`; a local-looking non-`.zig`
  path is `unsupported`, and a path escaping the analysis root is
  `outside_scope`.
- Bare package names are ignored: the analyzer does not resolve Zig package
  names, `build.zig` dependency declarations, or any package graph, and it does
  not execute a Zig build.
- `unused` treats `build.zig` at any directory depth, plus exact
  `src/main.zig`, `src/lib.zig`, and `src/root.zig` paths at the analysis root
  or under an exact `/src/...` suffix, as convention entry points. Arbitrary
  `.zig` files are not convention entries and need a resolved import edge.

Every edge carries `"confidence": "high"` by construction; the agent-json
projection drops it but keeps every `unresolved` reference, so agents never
mistake "unresolved" for "no dependency".

## Definitions

- `fan_in`: number of distinct files with a direct edge to this file
  (direct importers). A self-import counts once toward both fan-in and
  fan-out.
- `fan_out`: number of distinct resolved files this file imports.
- `blast_radius`: number of unique direct and transitive dependents of the
  impact target (reverse-BFS over edges, shortest distance per file; the
  target itself never re-appears, even through a cycle).
- `direct_dependents`: dependents at distance 1.
- `blast_radius_percent`: `blast_radius / (files_analyzed - 1) * 100` on a
  **0-100 scale** (`0.0` when `files_analyzed <= 1`).
- `cycles`: strongly connected components of at least two files (Kosaraju).
  A self-import alone is an edge, not a cycle. Impact `cycles` keeps only the
  components containing the target.

## Deterministic ordering

`files` by path; `edges` by `(source, target)`; `unresolved` by
`(source, specifier, line, reason)`; each cycle's `files` sorted, and the
cycle list sorted; impact `dependents` by `(distance, path)`. The same
repository, configuration, and analyzer version produce byte-identical JSON
across runs.

## Terminal output

`dependencies` prints `Dependencies (N files, M edges, K cycle[s])`, the top
5 files by fan-in and by fan-out, then `No cycles found.` or the numbered
`Cycles (K)` list. `impact` prints `Impact (target: ...)`, `Target`,
`Fan-in`, `Fan-out`, `Direct dependents`, `Blast radius: X of Y files (Z%)`,
the `Dependents (N shown, M total)` list with `(distance N)` per row
(`No dependents found.` when empty), `Showing the top N dependents (raise
--top for more).` when truncated, and `Cycles involving target (K)` when
the target sits on a cycle.

## Limitations

- No package-manager graph: third-party and bare imports are ignored.
- No path aliases or `tsconfig` paths: only relative specifiers resolve.
- No wildcard Java imports (`import pkg.*` is recorded, never resolved).
- No same-package implicit Java references: files using a type without an
  explicit import contribute no edge.
- No C compiler include paths or macro expansion: only exact quoted paths
  relative to the including file resolve; angle-bracket and macro-built
  includes are ignored.
- No Rust `use` or `extern crate` edges: those paths are module-qualified,
  never file-relative.
- No inline Rust modules as files: `mod foo { ... }` is not a file
  reference, but a `mod bar;` declared inside it resolves under `foo/`.
- No C++ system-include search path: angle-bracket includes are ignored.
- No Python `sys.path`, virtual-environment, or package-root knowledge:
  absolute imports (`import a.b`, `from a.b import c`) are recorded as
  `unsupported`, never resolved.
- No Python dynamic imports: `importlib.import_module(...)` and
  `__import__(...)` calls are invisible to the graph.
- No `.pyi` stubs: they declare signatures without bodies.
- No Python attribute-to-submodule resolution: `from .mod import name`
  references the module `mod` only, so a name that is really a submodule of
  a package resolves through that package's `__init__.py`, or not at all.
- No C++ preprocessor expansion: macro-built include paths are ignored.
- No method or function call graph: edges are file-level import/call-form
  evidence only (`call` means the `require()` / `import()` form, not a call
  graph).
- No reflection (`Class.forName`, dynamic proxies) or other runtime tricks.
- No runtime dynamic import expressions (variables, templates,
  concatenation): they are invisible to the graph.
- No generated-code special treatment beyond normal discovery excludes:
  generated and vendor directories are skipped by path-component names, and
  `leadline.toml` `[analysis] exclude` removes the rest from scope.
- Case-sensitive extension probing: an explicit specifier matching a
  discovered file resolves regardless of case, but extensionless and
  `index` probing only tries lowercase extensions.

Resolved imports are static evidence: they show what the source text names,
and they are incomplete wherever language or runtime configuration (aliases,
package graph, reflection, runtime-built specifiers) decides the real target.
Treat the graph as a starting point for inspection, not as proof of runtime
behavior.
