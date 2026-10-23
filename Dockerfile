# syntax=docker/dockerfile:1
#
# Z-UI as a container.
#
#   docker build -t z-ui .                        # builds everything from source
#   docker build -t z-ui --target prebuilt .      # packs the programs a release made (see release.yml)
#
# The panel keeps everything in /data, so that is the volume to keep. It
# listens on 2053 unless a ZUI_LISTEN says otherwise; run it with host
# networking, since the inbounds listen on ports chosen in the panel.

# ---- the dashboard
FROM node:24-bookworm-slim AS dashboard
WORKDIR /src/dashboard
COPY dashboard/package.json dashboard/package-lock.json ./
RUN npm ci
COPY dashboard/ ./
RUN npm run build

# ---- the panel, from source. zray-core, which it is built on, is fetched here.
FROM rust:1-bookworm AS builder
ARG CORE_REPOSITORY=https://github.com/ZeroNetTeam/Zray-Core.git
ARG CORE_REF=main
WORKDIR /src
RUN git clone --depth 1 --branch "${CORE_REF}" "${CORE_REPOSITORY}" resources/zray-core
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates crates
COPY --from=dashboard /src/dashboard/dist dashboard/dist
RUN cargo build --release -p z-panel && cp target/release/z-ui /z-ui

# ---- what runs: glibc and the certificate roots, and nothing else
FROM gcr.io/distroless/base-debian12 AS base
ENV ZUI_DATA_DIR=/data \
    ZUI_CONTAINER=1
WORKDIR /data
VOLUME /data
EXPOSE 2053
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
    CMD ["/usr/local/bin/z-ui", "health"]
ENTRYPOINT ["/usr/local/bin/z-ui"]

# The programs a release made, one for each kind of machine (dist/z-ui-linux-amd64, …-arm64).
FROM base AS prebuilt
ARG TARGETARCH
COPY dist/z-ui-linux-${TARGETARCH} /usr/local/bin/z-ui

# The default: built from source.
FROM base AS source
COPY --from=builder /z-ui /usr/local/bin/z-ui
