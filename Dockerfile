FROM --platform=$BUILDPLATFORM rust:1.88-bookworm AS builder

ARG TARGETARCH

WORKDIR /app

RUN rustup target add ${TARGETARCH}-unknown-linux-gnu

# TARGETARCH = {x86_64, aarch64}
COPY Cargo.toml Cargo.lock ./
RUN cargo fetch --target=${TARGETARCH}-unknown-linux-gnu

COPY . .
RUN cargo build --release --target=${TARGETARCH}-unknown-linux-gnu

FROM docker.io/debian:bookworm-slim

WORKDIR /root/
COPY --from=builder /app/target/${TARGETARCH}-unknown-linux-gnu/release/k8s-mc-discord .

ENTRYPOINT [ "./k8s-mc-discord" ]