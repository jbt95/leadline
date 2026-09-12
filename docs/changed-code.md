# Changed code

`leadline changed [--base REV] [--path PATH]` and `leadline diff [REV] [--path PATH]` compare one base revision against the working tree (tracked changes plus untracked files). Default base is `HEAD~1`.

## Pairing

1. Collect changed paths from `git diff --no-renames --name-only -z <base> --` plus untracked files from `git ls-files --others --exclude-standard -z`. Output is deterministic (sorted, NUL-delimited).
2. Skip files with unsupported extensions.
3. Parse the `before` bytes (`git show <base>:<path>`) and the `after` bytes (working tree) independently.
4. Group functions by name within each file, then pair by same-name source order: first `foo` before with first `foo` after, and so on.
5. Compare a deterministic source fingerprint per function. Identical fingerprints are omitted; only added, removed, or edited pairs are reported.
6. Sort output by path, then by the after (or before) start line.

## Renames

Rename detection is off (`--no-renames`). A renamed function surfaces as one removal (`after: null`) plus one addition (`before: null`). A renamed file surfaces as removals under the old path plus additions under the new path.

## Parse errors

Files with diagnostics on either side appear in `parse_errors` with `{ path, before, after }` diagnostic lists, even when their functions pair identically.
