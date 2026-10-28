#!/bin/sh
# Does the image do what it promises? Used by CI before anything is pushed.
#
#   scripts/smoke-test-image.sh <image>
#
# Requests are made from inside the container's own network namespace
# (`--network container:…`): it works whatever network the caller is on:
# a laptop, or a CI job talking to an isolated dind daemon.
set -eu
IMAGE="${1:?usage: smoke-test-image.sh <image>}"
NAME="ranch-smoke-$$"

# No password, no start.
if docker run --rm "$IMAGE" >/dev/null 2>&1; then
  echo "smoke: the image started without RANCH_PASSWORD" >&2
  exit 1
fi

docker run -d --name "$NAME" -e RANCH_PASSWORD=ci-secret "$IMAGE" >/dev/null
trap 'docker logs "$NAME" 2>&1 | tail -20 || true; docker rm -f "$NAME" >/dev/null' EXIT
req() { docker run --rm --network "container:$NAME" curlimages/curl:8.10.1 -s "$@"; }

for _ in $(seq 1 30); do
  req -f http://127.0.0.1:8080/healthz >/dev/null && break
  sleep 1
done
test "$(req http://127.0.0.1:8080/healthz)" = ok
# The UI and the API are behind the password…
test "$(req -o /dev/null -w '%{http_code}' http://127.0.0.1:8080/)" = 401
# …which opens them; an empty database has no connection.
test "$(req -u ranch:ci-secret -X POST -H 'x-ranch-request: 1' \
  -H 'content-type: application/json' -d '{}' \
  http://127.0.0.1:8080/api/invoke/list_connections)" = "[]"
# The API refuses what a foreign page could forge (no custom header).
test "$(req -o /dev/null -w '%{http_code}' -u ranch:ci-secret -X POST \
  http://127.0.0.1:8080/api/invoke/list_connections)" = 403
# The image's own health check agrees.
docker exec "$NAME" /usr/local/bin/ranch-server --healthcheck
trap 'docker rm -f "$NAME" >/dev/null' EXIT
echo "smoke: $IMAGE OK"
