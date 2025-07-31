FROM --platform=$BUILDPLATFORM ghcr.io/rust-cross/cargo-zigbuild AS builder

ARG TARGETARCH
# ARG RUST_TRIPLE # e.g., x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu

WORKDIR /app

# RUN apt-get update && \
#     apt-get install -y gcc-aarch64-linux-gnu && \
#     rm -rf /var/lib/apt/lists/*

# RUN rustup target add ${RUST_TRIPLE}

# TARGETARCH = {x86_64, aarch64}
COPY . .

RUN case "$TARGETARCH" in \
    amd64)  export RUST_TRIPLE=x86_64-unknown-linux-gnu ;; \
    arm64)  export RUST_TRIPLE=aarch64-unknown-linux-gnu ;; \
    *)      echo "Unsupported arch $TARGETARCH" >&2; exit 1 ;; \
    esac && \
    cargo zigbuild --release --target=$RUST_TRIPLE && \
    cp target/$RUST_TRIPLE/release/k8s-mc-discord /app/k8s-mc-discord

FROM docker.io/debian:bookworm-slim

ARG RUST_TRIPLE

WORKDIR /root/
COPY --from=builder /app/k8s-mc-discord .

ENTRYPOINT [ "./k8s-mc-discord" ]