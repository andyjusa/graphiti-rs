FROM rust:1.95-bookworm AS builder
WORKDIR /build
ARG CARGO_FEATURES=""
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked --features "${CARGO_FEATURES}"

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/* && useradd -u 10001 -m graphiti
COPY --from=builder /build/target/release/graphiti-rs /usr/local/bin/graphiti-rs
USER 10001
EXPOSE 8080
ENTRYPOINT ["graphiti-rs"]
CMD ["--transport", "http"]
