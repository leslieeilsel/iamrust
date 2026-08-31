.PHONY: dev dev-desktop build-desktop package-desktop dev-infra dev-infra-full dev-infra-down migrate seed check test lint format format-check build

dev:
	cargo run -p iamrust-desktop

dev-desktop:
	cargo run -p iamrust-desktop

build-desktop:
	cargo build --release -p iamrust-desktop

package-desktop:
	cargo packager --release --packages iamrust-desktop

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
	cargo check --workspace --all-targets

test:
	cargo test --workspace --all-targets

lint:
	cargo clippy --workspace --all-targets --all-features -- -D warnings

format:
	cargo fmt --all

format-check:
	cargo fmt --all --check

build:
	cargo build --workspace
