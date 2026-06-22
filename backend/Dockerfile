FROM rust:1.75-slim as builder

WORKDIR /usr/src/app

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.toml
RUN mkdir -p src && \
    echo "fn main() {}" > src/main.rs

RUN cargo build --release || true

COPY src/ src/

RUN cargo build --release

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/local/bin

COPY --from=builder /usr/src/app/target/release/urn-acoustics-backend .

EXPOSE 8080

CMD ["./urn-acoustics-backend"]
