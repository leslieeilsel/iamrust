#!/usr/bin/env sh
set -eu

dump_file="${1:?usage: restore-postgres.sh path/to/backup.dump}"
database_url="${IAMRUST_RESTORE_DATABASE_URL:?IAMRUST_RESTORE_DATABASE_URL is required}"

if [ ! -f "$dump_file" ]; then
  echo "Backup does not exist: $dump_file" >&2
  exit 1
fi

pg_restore --exit-on-error --clean --if-exists --no-owner --dbname="$database_url" "$dump_file"
echo "Restore completed and must now be verified with the smoke test."
