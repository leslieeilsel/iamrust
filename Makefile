.PHONY: dev dev-desktop build-desktop package-desktop dev-infra dev-infra-full dev-infra-down migrate seed check test lint format build

dev:
	pnpm dev

dev-desktop:
	pnpm dev:desktop

build-desktop:
	pnpm build:desktop

package-desktop:
	pnpm package:desktop

dev-infra:
	docker compose up -d postgres minio minio-init mailpit

dev-infra-full:
	docker compose --profile full up --build

dev-infra-down:
	docker compose --profile full down

migrate:
	cargo sqlx migrate run --source migrations

seed:
	./scripts/seed-dev.sh

check:
	pnpm check

test:
	pnpm test

lint:
	pnpm lint

format:
	pnpm format

build:
	pnpm build
