#!/usr/bin/env bash
# CI guard: a migration that may already be applied somewhere never changes.
# sqlx stores each applied migration's checksum and refuses to start when a
# file no longer matches it: editing a shipped migration, even a comment,
# stops every install that applied it. Change the schema with a new file.
#
#   scripts/check-migrations-unchanged.sh [base]     (default: origin/main)
#
# Adding migrations is always fine. The newest migration of `base` may still
# change while no release tag (v*) contains it: nobody outside has applied it.

set -euo pipefail

base="${1:-origin/main}"
dir="src-tauri/crates/sqlite/migrations"

git rev-parse --verify --quiet "$base^{commit}" >/dev/null || {
  echo "migrations: no \"$base\" to compare with (fetch it first)" >&2
  exit 1
}

newest=$(git ls-tree --name-only "$base" "$dir/" | grep '\.sql$' | sort | tail -n 1 || true)
changed=$(git diff --name-only --no-renames --diff-filter=MD "$base" -- "$dir" | grep '\.sql$' || true)

status=0
for file in $changed; do
  if [ "$file" = "$newest" ]; then
    added=$(git log --diff-filter=A --format=%H -n 1 "$base" -- "$file")
    released=$(git tag --list 'v*' --contains "$added")
    if [ -z "$released" ]; then
      echo "migrations: $file changed (newest, in no release yet: allowed)"
      continue
    fi
    echo "migrations: $file changed, but it shipped in $(echo "$released" | head -n 1)" >&2
  else
    echo "migrations: $file changed or removed" >&2
  fi
  status=1
done

if [ "$status" -ne 0 ]; then
  echo "A migration that has shipped never changes (sqlx checksums it): add a new one instead." >&2
  exit 1
fi
echo "migrations: no shipped migration changed"
