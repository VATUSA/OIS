# syntax=docker/dockerfile:1
#
# ois-backend — the Axum API. Multi-stage with cargo-chef so dependency compilation is cached
# in its own layer and only re-runs when Cargo.toml/Cargo.lock change. Migrations are embedded
# at compile time (sqlx::migrate!), so the runtime image needs neither the source nor a DB to
# build. Build context is the repo root: `docker build -f deploy/backend.Dockerfile .`

FROM rust:1-bookworm AS chef
RUN cargo install cargo-chef --locked
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
# Compile (and cache) just the dependencies.
RUN cargo chef cook --release --recipe-path recipe.json
# Then the workspace itself.
COPY . .
# Full version (1.0.1-<sha>) baked into the binary via option_env!("OIS_VERSION"). Set *after* the
# dependency cook so a new commit doesn't invalidate that cache layer — only the final crate rebuilds
# (which happens on any source change anyway).
ARG OIS_VERSION=""
ENV OIS_VERSION=$OIS_VERSION
RUN cargo build --release --bin ois-backend

FROM debian:bookworm-slim AS runtime
# ca-certificates: outbound HTTPS (VATSIM/VATUSA/SimAware). libssl3: sqlx's native-tls.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates libssl3 \
 && rm -rf /var/lib/apt/lists/*
RUN useradd --system --uid 10001 --user-group ois
COPY --from=builder /app/target/release/ois-backend /usr/local/bin/ois-backend
USER ois
# Bind on all interfaces inside the container (overridable).
ENV BIND_ADDR=0.0.0.0:3000
EXPOSE 3000
ENTRYPOINT ["ois-backend"]
