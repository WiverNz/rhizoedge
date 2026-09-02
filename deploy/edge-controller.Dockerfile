FROM rust:1.98-bookworm AS chef
RUN cargo install cargo-chef --locked --version 0.1.73
WORKDIR /src

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
# Empty in every production build. The M8 test overlay passes `e2e-faults`,
# which compiles in the process-boundary crash hooks SCEN-051 and SCEN-102 arm
# — a `std::process::exit` on the actuation path that must not exist in an image
# anybody could deploy.
ARG CARGO_FEATURES=""
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --locked --package edge-controller --recipe-path recipe.json
COPY . .
RUN cargo build --release --locked --package edge-controller \
    ${CARGO_FEATURES:+--features "$CARGO_FEATURES"}

FROM debian:bookworm-slim AS runtime
# `busybox-static` supplies the healthcheck's HTTP client. `kill -0 1` would
# report a wedged process as healthy, which is the one failure a health gate
# exists to catch — but `curl` and its TLS stack cost 12 MB and M8-001 caps a
# service image at 100 MB. `busybox wget` makes the same plaintext loopback
# request for about one, and nothing else in the image needs an HTTP client.
RUN apt-get update \
    && apt-get install --yes --no-install-recommends busybox-static \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home rhizo \
    && mkdir -p /var/lib/rhizo /etc/rhizo \
    && chown 10001:10001 /var/lib/rhizo
COPY --from=builder /src/target/release/edge-controller /usr/local/bin/edge-controller
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY deploy/edge/edge.toml /etc/rhizo/edge.toml
USER 10001:10001
WORKDIR /var/lib/rhizo
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/edge-controller", "--config", "/etc/rhizo/edge.toml"]
