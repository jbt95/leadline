#!/usr/bin/env bash
# Ensure CI has a Rust toolchain at least MSRV without redundant downloads.
set -euo pipefail

REQUIRED="${RUST_VERSION:-1.90}"
MAX_ATTEMPTS=3
MODE="${1:-minimum}"

version_ge() {
  local have="$1"
  local need="$2"
  local have_major have_minor have_patch
  local need_major need_minor need_patch
  IFS=. read -r have_major have_minor have_patch _ <<<"$have"
  IFS=. read -r need_major need_minor need_patch _ <<<"$need"
  have_minor="${have_minor:-0}"
  have_patch="${have_patch:-0}"
  need_minor="${need_minor:-0}"
  need_patch="${need_patch:-0}"
  if ((have_major > need_major)); then
    return 0
  fi
  if ((have_major < need_major)); then
    return 1
  fi
  if ((have_minor > need_minor)); then
    return 0
  fi
  if ((have_minor < need_minor)); then
    return 1
  fi
  ((have_patch >= need_patch))
}

current_rustc_version() {
  rustc --version 2>/dev/null | sed -n 's/^rustc \([0-9][0-9.]*\).*/\1/p' || true
}

install_with_retry() {
  local attempt delay
  for attempt in $(seq 1 "$MAX_ATTEMPTS"); do
    if rustup toolchain install "$REQUIRED" --profile minimal; then
      if [[ "$MODE" == "minimum" ]]; then
        rustup default "$REQUIRED"
      fi
      return 0
    fi
    if ((attempt < MAX_ATTEMPTS)); then
      delay=$((4 * (2 ** (attempt - 1))))
      echo "rustup toolchain install $REQUIRED failed (attempt $attempt/$MAX_ATTEMPTS); retrying in ${delay}s..."
      sleep "$delay"
    fi
  done
  echo "rustup toolchain install $REQUIRED failed after $MAX_ATTEMPTS attempts" >&2
  return 1
}

case "$MODE" in
  minimum)
    ver="$(current_rustc_version)"
    if [[ -n "$ver" ]] && version_ge "$ver" "$REQUIRED"; then
      echo "Runner rustc $ver already meets MSRV $REQUIRED; skipping toolchain download"
      exit 0
    fi
    install_with_retry
    ;;
  exact)
    install_with_retry
    ;;
  *)
    echo "usage: $0 [minimum|exact]" >&2
    exit 2
    ;;
esac
