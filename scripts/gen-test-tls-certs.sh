#!/usr/bin/env bash
# Generate a self-signed CA + server cert for local TLS Redis testing.
#
# Outputs to `.docker/tls/`:
#   ca.crt      — paste this into Ranch's "Custom CA certificate" field
#                 when adding a TLS connection to localhost:6380.
#   server.crt  — used by the `redis-tls` compose service.
#   server.key  — used by the `redis-tls` compose service.
#
# Validity: 30 days. Re-run to refresh. Idempotent: skips if the files
# already exist (delete `.docker/tls/{ca,server}.{crt,key}` to force).
#
# These certs are for LOCAL DEVELOPMENT ONLY. Do not deploy them anywhere.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TLS_DIR="$REPO_ROOT/.docker/tls"
mkdir -p "$TLS_DIR"
cd "$TLS_DIR"

if [ -f ca.crt ] && [ -f server.crt ] && [ -f server.key ]; then
  echo "✓ TLS certs already exist in .docker/tls/. Delete them to regenerate." >&2
  exit 0
fi

if ! command -v openssl >/dev/null 2>&1; then
  echo "ERROR: openssl not found in PATH. Install it (e.g. brew install openssl)." >&2
  exit 1
fi

echo "Generating local-dev TLS material in .docker/tls/ (30-day validity)..." >&2

# CA (only the cert is committed long-term; the key is throwaway after signing).
openssl genrsa -out ca.key 2048 2>/dev/null
openssl req -x509 -new -nodes -key ca.key -sha256 -days 30 \
  -subj "/CN=Ranch Local-Dev Test CA" -out ca.crt 2>/dev/null

# Server cert with SAN for both `localhost` and `127.0.0.1` so rustls
# verification succeeds whichever name Ranch dials.
openssl genrsa -out server.key 2048 2>/dev/null
openssl req -new -key server.key -subj "/CN=localhost" -out server.csr 2>/dev/null
cat > server.ext <<'EOF'
subjectAltName = DNS:localhost, IP:127.0.0.1
EOF
openssl x509 -req -in server.csr -CA ca.crt -CAkey ca.key -CAcreateserial \
  -out server.crt -days 30 -sha256 -extfile server.ext 2>/dev/null

# Cleanup intermediates. Keep ca.crt + server.{crt,key}; the rest is throwaway.
rm -f server.csr server.ext ca.srl ca.key

echo "✓ TLS certs ready:" >&2
echo "    .docker/tls/ca.crt      — paste into Ranch's Custom CA field" >&2
echo "    .docker/tls/server.crt  — used by docker compose redis-tls" >&2
echo "    .docker/tls/server.key  — used by docker compose redis-tls" >&2
echo "" >&2
echo "Next: docker compose --profile tls up -d" >&2
