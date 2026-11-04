#!/usr/bin/env bash
# Release pipelines: attach one platform's signed files to the tag's release.
#
#   scripts/attach-release.sh macos|linux <dir>
#
# release.yml's `attach` job runs it once per platform, on the files its
# signing job made (<dir>, already checked against that job's sums). It
# writes SHA256SUMS-<platform>.txt and uploads with `--clobber`: a re-run
# replaces what an earlier run attached, public release or not.
#
# Except the update the feed already serves. Once `publish` has written the
# platform's `platform-<platform>.json` on this release, installed copies are
# told to download this archive and check it against that signature: a re-run
# must not swap either. So the published archive and .sig are kept: they are
# downloaded into <dir> in place of the new ones (so the sums list them),
# checked against the app's public key, and not uploaded again. Everything
# else (the DMG, the .deb, the .rpm) is replaced as before.
#
# Environment: GH_TOKEN (contents: write), GH_REPO (owner/name), GITHUB_REF_NAME.
set -euo pipefail

platform=${1:-}
dir=${2:-}
tag=${GITHUB_REF_NAME:?GITHUB_REF_NAME is the tag}
die() {
  echo "::error::attach-release: $*"
  exit 1
}
[ -d "$dir" ] || die "usage: attach-release.sh macos|linux <dir>"
here=$(cd "$(dirname "$0")" && pwd)

case "$platform" in
  macos)
    feeds="darwin-aarch64 darwin-x86_64"
    updater=(Ranch.app.tar.gz)
    ;;
  linux)
    feeds="linux-x86_64"
    updater=()
    for f in "$dir"/*.AppImage; do [ -e "$f" ] && updater+=("$(basename "$f")"); done
    [ "${#updater[@]}" -le 1 ] || die "more than one AppImage in $dir"
    ;;
  *) die "usage: attach-release.sh macos|linux <dir>" ;;
esac
sums="SHA256SUMS-$platform.txt"

assets=$(gh release view "$tag" --json assets --jq '.assets[].name')
has_asset() { printf '%s\n' "$assets" | grep -Fxq -- "$1"; }

frozen=()
for p in $feeds; do
  if has_asset "platform-$p.json"; then
    # The feed serves this platform from this release: its update stays as published.
    if [ "$platform" = linux ] && [ "${#updater[@]}" -eq 0 ]; then
      updater=("$(printf '%s\n' "$assets" | grep -E '\.AppImage$' | head -1)")
    fi
    for f in ${updater[@]+"${updater[@]}"}; do
      [ -n "$f" ] || continue
      has_asset "$f" && has_asset "$f.sig" ||
        die "platform-$p.json is published but $f or $f.sig is not on the release: fix the release by hand"
      frozen+=("$f" "$f.sig")
    done
    break
  fi
done

if [ "${#frozen[@]}" -gt 0 ]; then
  kept=$(mktemp -d)
  for f in "${frozen[@]}"; do
    gh release download "$tag" --pattern "$f" --dir "$kept" --clobber
    [ -f "$kept/$f" ] || die "could not download the published $f"
  done
  node "$here/minisign.mjs" verify "$kept/${frozen[0]}" --signature "$kept/${frozen[1]}" ||
    die "the published ${frozen[0]} does not match its published signature"
  for f in "${frozen[@]}"; do
    if [ -f "$dir/$f" ] && ! cmp -s "$dir/$f" "$kept/$f"; then
      echo "::notice::$f: the feed already serves the published one; this run's is not attached"
    fi
    cp "$kept/$f" "$dir/$f"
  done
  rm -rf "$kept"
fi

rm -f "$dir/$sums"
(cd "$dir" && find . -maxdepth 1 -type f ! -name '.*' ! -name "$sums*" | sed 's|^\./||' | LC_ALL=C sort |
  xargs shasum -a 256 >"$sums.part" && mv "$sums.part" "$sums")
cat "$dir/$sums"

upload=()
for f in "$dir"/*; do
  name=$(basename "$f")
  skip=false
  for k in ${frozen[@]+"${frozen[@]}"}; do [ "$k" = "$name" ] && skip=true; done
  $skip || upload+=("$f")
done
gh release upload "$tag" "${upload[@]}" --clobber
