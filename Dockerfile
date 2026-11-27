# syntax=docker/dockerfile:1
FROM rust:1.85-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY src ./src
RUN cargo build --release --workspace --locked

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates ffmpeg curl \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=build /src/target/release/m7s-rs /usr/local/bin/m7s-rs
COPY config ./config
COPY web ./web
COPY docs ./docs
COPY README.md LICENSE ./
RUN mkdir -p /data/record && useradd -r -u 10001 m7s && chown -R m7s:m7s /app /data
USER m7s
EXPOSE 8180 1935 8554 8081 8082 8083 9000 4443 50051 9443
ENTRYPOINT ["m7s-rs"]
CMD ["--config", "/app/config/server.toml"]
