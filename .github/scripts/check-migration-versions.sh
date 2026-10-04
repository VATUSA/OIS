#!/usr/bin/env bash
# Fail if two migrations share a version number (VATUSA/OIS#569).
#
# sqlx neither dedups nor detects a duplicate: it applies both files, the second violates the primary
# key on `_sqlx_migrations.version`, and the backend fails to start with the database half-migrated.
# A duplicate never exists on one PR branch alone — only once two PRs that each picked "the next free
# number" have both merged — so this runs against the merge result: on every PR (GitHub checks out the
# merge ref), in `just ci`, and as a gate on the image build for `next`/`main`.
#
# A gap in the sequence is harmless; only a duplicate is fatal.
set -euo pipefail

dir="${1:-backend/migrations}"
shopt -s nullglob
files=("$dir"/*.sql)
if [ "${#files[@]}" -eq 0 ]; then
  echo "no migrations found in $dir" >&2
  exit 1
fi

# sqlx parses the prefix as an integer, so `90_a.sql` and `0090_b.sql` collide too — compare numbers,
# not strings. (No associative arrays: this also runs under macOS's stock bash 3.2 via `just ci`.)
listing=""
for f in "${files[@]}"; do
  name=$(basename "$f")
  prefix=${name%%_*}
  if ! [[ $prefix =~ ^[0-9]+$ ]]; then
    echo "not a versioned migration (sqlx needs NNNN_name.sql): $name" >&2
    exit 1
  fi
  listing+="$((10#$prefix)) $name"$'\n'
done
listing=$(printf '%s' "$listing" | sort -n)

dupes=$(printf '%s\n' "$listing" | cut -d' ' -f1 | uniq -d)
if [ -n "$dupes" ]; then
  echo "Duplicate migration version(s) — sqlx would apply both and half-migrate the database:" >&2
  for version in $dupes; do
    printf '%s\n' "$listing" | awk -v v="$version" '$1 == v { print "  " $1 "  " $2 }' >&2
  done
  echo "Renumber the newer one upward, past every open PR's highest (see AGENTS.md)." >&2
  exit 1
fi

echo "migration versions are unique (${#files[@]} files)"
