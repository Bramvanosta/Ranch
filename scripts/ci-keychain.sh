#!/usr/bin/env bash
# Release pipelines: the temporary keychain of release.yml's `sign-macos` job,
# which holds the Developer ID certificate while the app is signed.
#
#   scripts/ci-keychain.sh import <dir>   APPLE_CERTIFICATE (the .p12, base64) and
#                                         APPLE_CERTIFICATE_PASSWORD from the
#                                         environment; APPLE_SIGNING_IDENTITY must be
#                                         in it. Prints `keychain=<path>` for
#                                         $GITHUB_OUTPUT.
#   scripts/ci-keychain.sh delete <dir>   the search list put back as it was, the
#                                         keychain deleted, every file of <dir>
#                                         removed even when `security` fails. Safe
#                                         after a failed import, and twice.
#
# <dir> is the job's own folder ($RUNNER_TEMP/signing): the keychain and the
# saved search list live there. GitHub Actions only (GITHUB_ACTIONS=true): it
# changes the user's keychain search list, and nothing else of the login
# keychain (no default keychain, no import, no setting).
#
# No password is ever on a command line, which any process of the same user
# can read: /usr/bin/openssl (the system's, never one on PATH) reads the
# certificate's from the environment (`-passin env:`) and wraps the identity
# again under a random one-time password; that one and the keychain's own (random too) reach `security` on
# its standard input (`security -i`, one command per run). The private key is
# imported non-extractable (`-x`): codesign may use it, nothing can export it.
set -euo pipefail

cmd=${1:-}
dir=${2:-}
die() {
  echo "ci-keychain: $*" >&2
  exit 1
}
[ "${GITHUB_ACTIONS:-}" = true ] || die "GitHub Actions only (GITHUB_ACTIONS=true): it changes the keychain search list"
case "$cmd" in import | delete) ;; *) die "usage: ci-keychain.sh import|delete <dir>" ;; esac
case "$dir" in /*) ;; *) die "<dir> must be an absolute path (\$RUNNER_TEMP/signing)" ;; esac
# `security -i` splits its line on white space: the path must hold none, nor a quote.
case "$dir" in *[[:space:]\"\'\\]*) die "<dir> must hold no space, quote or backslash" ;; esac
keychain="$dir/ranch-signing.keychain-db"
saved="$dir/search-list.txt"
kc_pass=""
RANCH_WRAP_PASS=""

# The user's search list, one keychain per line (security prints each quoted and indented).
search_list() {
  security list-keychains -d user | sed -e 's/^[[:space:]]*"//' -e 's/"[[:space:]]*$//'
}

# A security command holding a password, sent on security's standard input.
# Fails when security exits non-zero or prints an error ("security: …"),
# showing the end of what it said with the passwords taken out. Everything
# goes to stderr: stdout is the step's $GITHUB_OUTPUT.
security_stdin() {
  local said status=0
  said=$(printf '%s\n' "$1" | security -i 2>&1) || status=$?
  [ -z "$kc_pass" ] || said=${said//"$kc_pass"/<redacted>}
  [ -z "$RANCH_WRAP_PASS" ] || said=${said//"$RANCH_WRAP_PASS"/<redacted>}
  said=$(printf '%s\n' "$said" | sed -e 's/^\(security> *\)*//' -e '/^[[:space:]]*$/d')
  if [ "$status" -ne 0 ] || printf '%s\n' "$said" | grep -q '^security: '; then
    printf '%s\n' "$said" | tail -n 4 >&2
    die "security ${1%% *} failed"
  fi
  if [ "${2:-}" != quiet ] && [ -n "$said" ]; then printf '%s\n' "$said" >&2; fi
}

if [ "$cmd" = delete ]; then
  status=0
  if [ -f "$saved" ]; then
    list=()
    while IFS= read -r k; do
      if [ -n "$k" ] && [ "$k" != "$keychain" ]; then list+=("$k"); fi
    done <"$saved"
    security list-keychains -d user -s ${list[@]+"${list[@]}"} || status=$?
  fi
  if [ -e "$keychain" ]; then security delete-keychain "$keychain" || status=$?; fi
  # Whatever security said, no file of the signing material stays.
  if [ -d "$dir" ]; then find "$dir" -mindepth 1 -delete; fi
  [ "$status" -eq 0 ] || die "security failed while the keychain was removed (its files are gone; the search list may still name it)"
  exit 0
fi

[ -n "${APPLE_CERTIFICATE:-}" ] || die "APPLE_CERTIFICATE is empty (the Developer ID Application certificate's .p12, base64)"
[ -n "${APPLE_CERTIFICATE_PASSWORD:-}" ] || die "APPLE_CERTIFICATE_PASSWORD is empty (the password the .p12 was exported with)"
[ -n "${APPLE_SIGNING_IDENTITY:-}" ] || die "APPLE_SIGNING_IDENTITY is empty (\"Developer ID Application: <name> (<team id>)\")"
[ ! -e "$keychain" ] && [ ! -e "$saved" ] || die "$dir holds a keychain already: run delete first"
umask 077
mkdir -p "$dir"
chmod 700 "$dir"
p12="$dir/certificate.p12"
wrapped="$dir/wrapped.p12"
trap 'rm -f "$p12" "$wrapped"' EXIT
printf '%s' "$APPLE_CERTIFICATE" | base64 --decode >"$p12" 2>/dev/null ||
  die "APPLE_CERTIFICATE is not base64 (base64 -i certificate.p12)"
kc_pass=$(/usr/bin/openssl rand -hex 24)
RANCH_WRAP_PASS=$(/usr/bin/openssl rand -hex 24)
export RANCH_WRAP_PASS
# The certificates and the key each through a pipe of their own, never a file in the clear.
/usr/bin/openssl pkcs12 -export -passout env:RANCH_WRAP_PASS -out "$wrapped" \
  -in <(/usr/bin/openssl pkcs12 -in "$p12" -passin env:APPLE_CERTIFICATE_PASSWORD -nokeys 2>/dev/null) \
  -inkey <(/usr/bin/openssl pkcs12 -in "$p12" -passin env:APPLE_CERTIFICATE_PASSWORD -nocerts -nodes 2>/dev/null) 2>/dev/null ||
  die "the .p12 could not be read with APPLE_CERTIFICATE_PASSWORD"

# The search list before create-keychain adds the new keychain to it.
search_list >"$saved"
security_stdin "create-keychain -p $kc_pass $keychain"
[ -f "$keychain" ] || die "security create-keychain made no keychain"
# Locks again after six hours (longer than the job may run), never on sleep.
security set-keychain-settings -lut 21600 "$keychain" >&2
security_stdin "unlock-keychain -p $kc_pass $keychain"
security_stdin "import $wrapped -k $keychain -f pkcs12 -x -P $RANCH_WRAP_PASS -T /usr/bin/codesign"
# codesign may use the key without a prompt (nobody is there to answer one).
security_stdin "set-key-partition-list -S apple-tool:,apple:,codesign: -s -k $kc_pass $keychain" quiet
list=("$keychain")
while IFS= read -r k; do
  if [ -n "$k" ] && [ "$k" != "$keychain" ]; then list+=("$k"); fi
done <"$saved"
security list-keychains -d user -s "${list[@]}" >&2
identities=$(security find-identity -v -p codesigning "$keychain")
if ! printf '%s\n' "$identities" | grep -Fq "\"$APPLE_SIGNING_IDENTITY\""; then
  die "the certificate holds no valid code-signing identity named as APPLE_SIGNING_IDENTITY (check the secret, that the .p12 holds the private key, and that it holds Apple's Developer ID intermediate certificate: docs/releasing.md)"
fi
echo "keychain=$keychain"
