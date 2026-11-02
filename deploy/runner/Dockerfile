# syntax=docker/dockerfile:1
#
# Container image of the managed (cloud-agent) runner. Build it from the
# repository root: docker build -f deploy/runner/Dockerfile -t <name> .
# See crates/runner/README.md, "Container image".

# Keep the Rust version equal to rust-toolchain.toml.
FROM rust:1.98.0-bookworm AS build
RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential pkg-config clang cmake libssl-dev \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
RUN cargo build --locked --release -p nocterm-runner \
    && install -D target/release/nocterm-runner /out/nocterm-runner

FROM debian:bookworm-slim
# The runner contains tool execution with bwrap and prlimit (/usr/bin) and runs
# commands with bash.
RUN apt-get update \
    && apt-get install -y --no-install-recommends bubblewrap util-linux bash ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 10001 --create-home --shell /usr/sbin/nologin runner
COPY --from=build /out/nocterm-runner /usr/local/bin/nocterm-runner
USER 10001:10001
WORKDIR /home/runner
ENTRYPOINT ["/usr/local/bin/nocterm-runner"]
