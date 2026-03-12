FROM rust:1.94-trixie as builder

WORKDIR /usr/src/app

COPY Cargo.toml Cargo.lock* ./
COPY src ./src
COPY tests ./tests

RUN cargo test && cargo build --release

FROM debian:trixie-slim

WORKDIR /app

RUN apt-get update && apt-get install -y libssl-dev ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/src/app/target/release/age-inbox-cli /usr/local/bin/age-inbox-cli

RUN mkdir -p /app/downloads
ENV DOWNLOADS_DIR=/app/downloads

# Usually a CLI runs ad-hoc, but keeping this as standard setup.
CMD ["age-inbox-cli", "--help"]
