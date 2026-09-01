FROM rust:1.98-bookworm AS chef
RUN cargo install cargo-chef --locked --version 0.1.73
WORKDIR /src

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --locked --package rhizo-scenarios --recipe-path recipe.json
COPY . .
RUN cargo build --release --locked --package rhizo-scenarios

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install --yes --no-install-recommends docker.io ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home rhizo \
    && mkdir -p /artifacts \
    && chown 10001:10001 /artifacts
COPY --from=builder /src/target/release/scenario-runner /usr/local/bin/scenario-runner
USER 10001:10001
ENTRYPOINT ["/usr/local/bin/scenario-runner"]
