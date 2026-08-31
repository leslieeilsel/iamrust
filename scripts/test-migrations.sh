#!/usr/bin/env sh
set -eu

admin_url="${IAMRUST_POSTGRES_ADMIN_URL:?IAMRUST_POSTGRES_ADMIN_URL is required}"
fresh_url="${admin_url%/*}/iamrust_migration_fresh"
upgrade_url="${admin_url%/*}/iamrust_migration_upgrade"
restore_url="${admin_url%/*}/iamrust_migration_restore"
temporary_directory="$(mktemp -d)"

cleanup() {
  psql "$admin_url" -v ON_ERROR_STOP=1 -c 'DROP DATABASE IF EXISTS iamrust_migration_fresh WITH (FORCE)' >/dev/null
  psql "$admin_url" -v ON_ERROR_STOP=1 -c 'DROP DATABASE IF EXISTS iamrust_migration_upgrade WITH (FORCE)' >/dev/null
  psql "$admin_url" -v ON_ERROR_STOP=1 -c 'DROP DATABASE IF EXISTS iamrust_migration_restore WITH (FORCE)' >/dev/null
  rm -r "$temporary_directory"
}
trap cleanup EXIT INT TERM

create_database() {
  database_name="$1"
  psql "$admin_url" -v ON_ERROR_STOP=1 -c "DROP DATABASE IF EXISTS ${database_name} WITH (FORCE)" >/dev/null
  psql "$admin_url" -v ON_ERROR_STOP=1 -c "CREATE DATABASE ${database_name}" >/dev/null
}

create_database iamrust_migration_fresh
for migration in migrations/*.sql; do
  psql "$fresh_url" -v ON_ERROR_STOP=1 -f "$migration" >/dev/null
done

create_database iamrust_migration_upgrade
psql "$upgrade_url" -v ON_ERROR_STOP=1 -f migrations/0001_initial.sql >/dev/null
psql "$upgrade_url" -v ON_ERROR_STOP=1 -f migrations/0002_search_and_retention.sql >/dev/null
for migration in migrations/0003_*.sql migrations/0004_*.sql migrations/0005_*.sql; do
  [ -f "$migration" ] || continue
  psql "$upgrade_url" -v ON_ERROR_STOP=1 -f "$migration" >/dev/null
done

IAMRUST_DATABASE_URL="$fresh_url" IAMRUST_BACKUP_DIR="$temporary_directory" ./scripts/backup-postgres.sh >/dev/null
dump_file="$(find "$temporary_directory" -name '*.dump' -type f -print -quit)"
create_database iamrust_migration_restore
IAMRUST_RESTORE_DATABASE_URL="$restore_url" ./scripts/restore-postgres.sh "$dump_file" >/dev/null

fresh_tables="$(psql "$fresh_url" -Atc "SELECT count(*) FROM information_schema.tables WHERE table_schema='public'")"
upgrade_tables="$(psql "$upgrade_url" -Atc "SELECT count(*) FROM information_schema.tables WHERE table_schema='public'")"
restore_tables="$(psql "$restore_url" -Atc "SELECT count(*) FROM information_schema.tables WHERE table_schema='public'")"
[ "$fresh_tables" = "$upgrade_tables" ]
[ "$fresh_tables" = "$restore_tables" ]
echo "Migration and restore verification passed (${fresh_tables} public tables)."
