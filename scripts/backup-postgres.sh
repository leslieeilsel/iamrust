#!/usr/bin/env sh
set -eu

backup_dir="${IAMRUST_BACKUP_DIR:-./backups}"
database_url="${IAMRUST_DATABASE_URL:?IAMRUST_DATABASE_URL is required}"
timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$backup_dir"
umask 077
pg_dump --format=custom --no-owner --file="${backup_dir}/iamrust-${timestamp}.dump" "$database_url"
sha256sum "${backup_dir}/iamrust-${timestamp}.dump" > "${backup_dir}/iamrust-${timestamp}.dump.sha256"
echo "Backup created: ${backup_dir}/iamrust-${timestamp}.dump"
