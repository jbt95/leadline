# Changed code

`leadline changed [--base REV] [--staged | --target REV] [--renames] [--path PATH]` and `leadline diff [REV] [--staged | --target REV] [--renames] [--path PATH]` compare one base revision against a comparison target. Default base is `HEAD~1`; default target is the working tree (tracked changes plus untracked files). `--staged` compares against the index (staged blobs) instead; `--target REV` compares two Git trees instead. `--staged` and `--target` are mutually exclusive.

## Pairing

1. Collect changed paths from `git diff [--cached] [-M|--no-renames] --name-status -z <base> [<target>] --`, plus untracked files from `git ls-files --others --exclude-standard -z` when the target is the working tree. Output is deterministic (sorted, NUL-delimited).
2. Skip files with unsupported extensions.
3. Parse the `before` bytes (`git show <base>:<path>`, or the pre-rename path when `--renames` paired a rename) and the `after` bytes independently: the working tree file for the default target, `git show :<path>` (staged blob) for `--staged`, or `git show <target>:<path>` for `--target REV`.
4. Group functions by name within each file, then pair by same-name source order: first `foo` before with first `foo` after, and so on.
5. Compare a deterministic source fingerprint per function. Identical fingerprints are omitted; only added, removed, or edited pairs are reported.
6. Sort output by path, then by the after (or before) start line.

## Targets

- Default (worktree): tracked changes plus untracked files, with rename detection off. `check --base REV` gates this same enumeration.
- `--staged` (index): compares `base` against the index only; unstaged working-tree edits are excluded.
- `--target REV`: compares `base` against `REV` as two Git trees; the working tree and index are never read.

`--base` and `--target` values are validated the same way: empty, option-like (leading `-`), or control-character revisions are rejected before any Git subprocess runs.

## Renames

Rename detection is off by default (`--no-renames`). A renamed function surfaces as one removal (`after: null`) under the old path plus one addition (`before: null`) under the new path.

With `--renames`, Git file rename detection (`-M`) pairs a renamed file's pre-rename content with its post-rename content under the new path; function matching within the pair remains exact name plus source order (no fuzzy matching). A function unchanged by the rename is omitted like any other unchanged pair; a function edited during the rename surfaces once, under the new path, with both `before` and `after` populated.

## Parse errors

Files with diagnostics on either side appear in `parse_errors` with `{ path, before, after }` diagnostic lists, even when their functions pair identically.
