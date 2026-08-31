FROM rust:1.98-bookworm AS builder
WORKDIR /workspace

COPY Cargo.toml Cargo.lock rust-toolchain.toml rustfmt.toml ./
COPY crates ./crates
COPY apps/server ./apps/server
COPY apps/desktop/src-tauri/Cargo.toml ./apps/desktop/src-tauri/Cargo.toml
RUN mkdir -p apps/desktop/src-tauri/src && \
    printf 'fn main() {}\n' > apps/desktop/src-tauri/src/main.rs && \
    printf 'pub fn run() {}\n' > apps/desktop/src-tauri/src/lib.rs && \
    printf 'fn main() {}\n' > apps/desktop/src-tauri/build.rs
RUN cargo build --locked --release -p iamrust-server

FROM debian:bookworm-slim AS runtime
RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates curl && \
    rm -rf /var/lib/apt/lists/*
RUN useradd --create-home --uid 10001 iamrust
COPY --from=builder /workspace/target/release/iamrust-server /usr/local/bin/iamrust-server
USER iamrust
EXPOSE 3780
ENTRYPOINT ["/usr/local/bin/iamrust-server"]
