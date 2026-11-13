# syntax=docker/dockerfile:1
# The Zecret maker daemon (`zecret maker`, crates/zecret-node) as a container.
#
# Build from the repository root (the build context is the whole repository; the file
# Dockerfile.dockerignore next to this one keeps target directories and other large
# folders out of it):
#
#   docker build -f deploy/maker/Dockerfile -t zecret-maker .
#
# Run with the maker's directory (config.json, near_key.json, wallet and swap records)
# on a volume:
#
#   docker run -d --name zecret-maker --restart unless-stopped \
#     -v /srv/zecret-maker:/data zecret-maker
#
# See deploy/maker/README.md.

ARG RUST_VERSION=1.97
ARG DEBIAN_RELEASE=bookworm

FROM rust:${RUST_VERSION}-slim-${DEBIAN_RELEASE} AS build
# The crate path and binary name, in case they move.
ARG CRATES_DIR=crates
ARG PACKAGE=zecret-node
ARG BIN=zecret
RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config libssl-dev ca-certificates protobuf-compiler \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY ${CRATES_DIR}/ ${CRATES_DIR}/
# Behind a TLS-intercepting proxy, pass its CA bundle as a build secret:
#   docker build --secret id=cacert,src=/path/ca.crt --build-arg HTTPS_PROXY=... ...
# Without the secret, cargo uses the image's own CA store.
RUN --mount=type=secret,id=cacert,required=false \
    --mount=type=cache,target=/usr/local/cargo/registry \
    if [ -f /run/secrets/cacert ]; then export CARGO_HTTP_CAINFO=/run/secrets/cacert; fi \
 && cd ${CRATES_DIR} \
 && cargo build --release --locked -p ${PACKAGE} --bin ${BIN} \
 && install -m 0755 target/release/${BIN} /usr/local/bin/zecret

FROM debian:${DEBIAN_RELEASE}-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates tini \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --home-dir /data --shell /usr/sbin/nologin zecret \
 && mkdir -p /data && chown zecret /data && chmod 0700 /data
COPY --from=build /usr/local/bin/zecret /usr/local/bin/zecret
USER zecret
VOLUME ["/data"]
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/zecret"]
CMD ["maker", "--dir", "/data"]
