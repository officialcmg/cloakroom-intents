#!/usr/bin/env bash
# Vercel "Ignored Build Step" for the website (docs/HOSTING.md).
# Exit 0: skip the build. Exit 1: build.
#
# It builds when a file changed, since the last successful deployment of this branch,
# under web/, under the crates that the browser engine is built from, or in deploy/web/.
# When the previous deployment's commit is unknown or not in the clone, it builds.
set -u
cd "$(git rev-parse --show-toplevel)" || exit 1
prev="${VERCEL_GIT_PREVIOUS_SHA:-}"
if [ -z "$prev" ]; then
  echo "No previous deployment on this branch: build."
  exit 1
fi
if ! git cat-file -e "$prev^{commit}" 2>/dev/null; then
  git fetch --quiet --depth=50 origin "$prev" 2>/dev/null || true
fi
if ! git cat-file -e "$prev^{commit}" 2>/dev/null; then
  echo "Commit $prev of the previous deployment is not in the clone: build."
  exit 1
fi
paths="web crates/zecret-wasm crates/zecret-core crates/zecret-intents crates/Cargo.toml crates/Cargo.lock crates/.cargo deploy/web"
# shellcheck disable=SC2086
if git diff --quiet "$prev" HEAD -- $paths; then
  echo "No change under $paths since $prev: skip."
  exit 0
fi
echo "Changes since $prev:"
# shellcheck disable=SC2086
git diff --stat "$prev" HEAD -- $paths | tail -n 20
exit 1
