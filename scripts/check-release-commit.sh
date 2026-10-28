#!/usr/bin/env bash
# Release guard: a version tag ships only a commit of main whose CI passed.
# release.yml and image.yml run it before they build anything for a tag.
#
#   scripts/check-release-commit.sh <sha>
#
# Environment: GH_TOKEN (contents: read, actions: read), GH_REPO (owner/name).
# A tag pushed right after main finds that commit's CI still running: it waits
# for it, up to WAIT_MINUTES (25 by default), then says to re-run.

set -euo pipefail

sha="${1:?usage: check-release-commit.sh <sha>}"
wait_minutes="${WAIT_MINUTES:-25}"

# On main: the commit is main's head or one of its ancestors.
status=$(gh api "repos/$GH_REPO/compare/main...$sha" --jq .status)
case "$status" in
  identical | behind) echo "release: $sha is on main" ;;
  *)
    echo "::error::$sha is not on main (compare: $status). Tag a commit of main."
    exit 1
    ;;
esac

# Its CI: the run of ci.yml for that push to main.
deadline=$(($(date +%s) + wait_minutes * 60))
while :; do
  run=$(gh run list --workflow ci.yml --branch main --event push --commit "$sha" --limit 1 \
    --json status,conclusion,url --jq '.[0] // empty | "\(.status) \(.conclusion) \(.url)"')
  read -r run_status conclusion url <<<"${run:-none none none}"
  if [ "$run_status" = completed ]; then
    if [ "$conclusion" = success ]; then
      echo "release: CI passed on $sha ($url)"
      exit 0
    fi
    echo "::error::CI did not pass on $sha ($conclusion): $url"
    exit 1
  fi
  if [ "$(date +%s)" -ge "$deadline" ]; then
    echo "::error::CI on $sha is not done ($run_status) after ${wait_minutes} min: re-run this job once it is. $url"
    exit 1
  fi
  echo "release: CI on $sha is $run_status, waiting"
  sleep 30
done
