FROM mcr.microsoft.com/devcontainers/rust:2-1-trixie

RUN apt-get update \
    && apt-get install -y protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

ENV CARGO_HOME=/usr/local/cargo RUSTUP_HOME=/usr/local/rustup
