#!/usr/bin/env bash
# CI guard: fail if any *.sql migration introduces a column whose name suggests
# it stores credentials. Credentials must live in the OS keychain only
#.
#
# Allowed: `keychain_ref` (a non-credential reference to a keychain entry name).

set -euo pipefail

forbidden_patterns='\b(password|passwd|secret|credential|api[-_]?key|auth[-_]?token|bearer)\b'
target_dir="${1:-src-tauri/crates/sqlite/migrations}"

if [[ ! -d "$target_dir" ]]; then
  echo "error: directory not found: $target_dir" >&2
  exit 2
fi

matches=$(grep -rEnIi "$forbidden_patterns" "$target_dir" || true)

if [[ -n "$matches" ]]; then
  echo "❌ Forbidden credential-like column names found in migrations:" >&2
  echo "$matches" | sed 's/^/   /' >&2
  echo "" >&2
  echo "Credentials must live in the OS keychain only." >&2
  echo "Store the keychain entry name in a non-credential column (e.g. 'keychain_ref')." >&2
  exit 1
fi

echo "✅ no-creds-in-sql guard: clean"
