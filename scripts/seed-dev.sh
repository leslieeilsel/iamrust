#!/usr/bin/env sh
set -eu

api_url="${IAMRUST_API_URL:-http://127.0.0.1:3780}"

register() {
  username="$1"
  curl --fail --silent --show-error \
    -H 'content-type: application/json' \
    -d "{\"email\":\"${username}@example.test\",\"username\":\"${username}\",\"password\":\"Development1\",\"nickname\":\"${username}\",\"device_name\":\"seed-script\"}" \
    "${api_url}/api/v1/auth/register"
}

register alice > /tmp/iamrust-alice-session.json || true
register bob > /tmp/iamrust-bob-session.json || true

echo "Development accounts prepared: alice / bob (password: Development1)"
echo "Session responses, when newly created, are in /tmp/iamrust-*-session.json"
