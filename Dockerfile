FROM rust:1.98.1-slim-trixie AS builder

RUN apt-get update &&  \
    apt-get install -y --no-install-recommends protobuf-compiler libprotobuf-dev &&  \
    rm -rf /var/lib/apt/lists/*

WORKDIR /work

COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/work/target \
    cargo build --release && \
    cp target/release/loop-csi-provisioner /loop-csi-provisioner

FROM debian:trixie-slim

LABEL org.opencontainers.image.url="https://github.com/SiLeader/loop-csi-provisioner" \
      org.opencontainers.image.licenses="Apache-2.0"

# mount + nfs-common: NFS mounts (mount.nfs); e2fsprogs: mkfs.ext4 and resize2fs
RUN apt-get update && \
    apt-get install -y --no-install-recommends mount nfs-common e2fsprogs && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /loop-csi-provisioner /usr/sbin/loop-csi-provisioner

ENTRYPOINT ["/usr/sbin/loop-csi-provisioner"]
