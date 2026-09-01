FROM rust:1.98-bookworm AS chef
RUN cargo install cargo-chef --locked --version 0.1.73
WORKDIR /src

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --locked --package cloud-api --recipe-path recipe.json
COPY . .
RUN cargo build --release --locked --package cloud-api

FROM debian:bookworm-slim AS runtime
RUN useradd --system --uid 10001 --no-create-home rhizo
COPY --from=builder /src/target/release/cloud-api /usr/local/bin/cloud-api
USER 10001:10001
EXPOSE 8081
ENTRYPOINT ["/usr/local/bin/cloud-api"]
