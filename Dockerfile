FROM rust:1.94-trixie as builder

WORKDIR /usr/src/app

RUN apt-get update && apt-get install -y mingw-w64 && rm -rf /var/lib/apt/lists/*
RUN rustup target add x86_64-pc-windows-gnu

ENV CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc

COPY Cargo.toml Cargo.lock* ./
COPY src ./src
COPY tests ./tests

# Build for linux
RUN cargo test && cargo build --release
# Build for windows
RUN cargo build --release --target x86_64-pc-windows-gnu

FROM debian:trixie-slim

WORKDIR /app

RUN apt-get update && apt-get install -y libssl-dev ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/src/app/target/release/age-inbox-cli /usr/local/bin/age-inbox-cli
COPY --from=builder /usr/src/app/target/x86_64-pc-windows-gnu/release/age-inbox-cli.exe /usr/local/bin/age-inbox-cli.exe

RUN mkdir -p /app/downloads
ENV DOWNLOADS_DIR=/app/downloads

# Usually a CLI runs ad-hoc, but keeping this as standard setup.
CMD ["age-inbox-cli", "--help"]
