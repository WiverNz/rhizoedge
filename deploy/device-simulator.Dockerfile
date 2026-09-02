FROM rust:1.98-bookworm AS chef
RUN cargo install cargo-chef --locked --version 0.1.73
WORKDIR /src

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --locked --package device-simulator --recipe-path recipe.json
COPY . .
RUN cargo build --release --locked --package device-simulator

FROM debian:bookworm-slim AS runtime
# `busybox-static` supplies the healthcheck's HTTP client, which asks the
# simulator's control API what it is actually serving rather than whether PID 1
# still exists. About 1 MB against 12 MB for `curl` and its TLS stack; M8-001
# caps a service image at 100 MB.
RUN apt-get update \
    && apt-get install --yes --no-install-recommends busybox-static \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home rhizo \
    && mkdir -p /var/lib/rhizo \
    && chown 10001:10001 /var/lib/rhizo
COPY --from=builder /src/target/release/device-simulator /usr/local/bin/device-simulator
# Resolves this replica's `device_id` before exec'ing the simulator, so a
# `--scale`d service produces distinct, separately authenticated devices
# (ADR-012). It `exec`s, so PID 1 is still the binary and SIGTERM still reaches
# the graceful-shutdown path M8-001 verifies.
COPY --chmod=0555 deploy/device-simulator-entrypoint.sh /usr/local/bin/device-simulator-entrypoint
USER 10001:10001
WORKDIR /var/lib/rhizo
ENTRYPOINT ["/usr/local/bin/device-simulator-entrypoint"]
