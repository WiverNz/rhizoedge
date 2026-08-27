# Rhizo Edge — the reference device simulator.
#
# PRD 020's acceptance criterion is that `docker compose up mosquitto
# device-simulator` runs standalone, with telemetry visible on the broker. This
# image is what makes that true, and it is also the image M8's scenario suite
# scales to several devices.
#
# Built from the workspace root so the shared crates come along: the simulator
# depends on `rhizo-mqtt-contract` and `rhizo-policy` by path, which is the
# mechanism ADR-008 uses to guarantee there is exactly one copy of the protocol
# and the safety rules on disk.

# The toolchain version lives in `rust-toolchain.toml` and nowhere else; the
# official image ships rustup, which honours it. Pinning a version here as well
# would give the repository two answers to the same question.
FROM rust:slim AS build

WORKDIR /src

# `--locked` so the image is built from the committed lockfile. Without it a
# container build could silently resolve a different dependency tree from the
# one the tests ran against, which is the sort of difference that only shows up
# in the environment you cannot debug.
COPY . .
RUN cargo build --release --locked -p device-simulator

# A distroless-style runtime: the binary and the CA bundle, nothing else. The
# simulator opens one TCP connection and writes one JSON file; a shell and a
# package manager in the image would be attack surface for no benefit.
FROM debian:stable-slim

RUN useradd --system --create-home --uid 10001 rhizo \
    && mkdir -p /var/lib/rhizo \
    && chown rhizo:rhizo /var/lib/rhizo

COPY --from=build /src/target/release/device-simulator /usr/local/bin/device-simulator

USER rhizo
WORKDIR /var/lib/rhizo

# No default `--device-id`: every device must be named deliberately. The
# username the broker authenticates is the device id (ADR-012), so a default
# here would be a default identity, and two containers started without thinking
# would fight over one identity and evict each other.
ENTRYPOINT ["device-simulator"]
