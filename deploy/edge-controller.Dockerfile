FROM rust:1.98-bookworm AS chef
RUN cargo install cargo-chef --locked --version 0.1.73
WORKDIR /src

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --locked --package edge-controller --recipe-path recipe.json
COPY . .
RUN cargo build --release --locked --package edge-controller

FROM debian:bookworm-slim AS runtime
RUN useradd --system --uid 10001 --no-create-home rhizo \
    && mkdir -p /var/lib/rhizo /etc/rhizo \
    && chown 10001:10001 /var/lib/rhizo
COPY --from=builder /src/target/release/edge-controller /usr/local/bin/edge-controller
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY deploy/edge/edge.toml /etc/rhizo/edge.toml
USER 10001:10001
WORKDIR /var/lib/rhizo
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/edge-controller", "--config", "/etc/rhizo/edge.toml"]
