FROM rust:1.98-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --locked --release -p cloud-api
FROM debian:bookworm-slim
RUN useradd --system --uid 10001 rhizo
COPY --from=build /src/target/release/cloud-api /usr/local/bin/cloud-api
USER rhizo
EXPOSE 8081
ENTRYPOINT ["cloud-api"]
