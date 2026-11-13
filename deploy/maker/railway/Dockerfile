# syntax=docker/dockerfile:1
# The hosted testnet maker with its relay, as Railway builds it (docs/HOSTING.md).
#
# Railway builds this file from the repository root on every push to main that changes
# crates/** or deploy/maker/** (the service's watch paths). It differs from
# deploy/maker/Dockerfile in three ways:
#   - no BuildKit cache or secret mounts (Railway accepts cache mounts only with an id
#     that names the service);
#   - the start script deploy/maker/railway/start.sh: it prepares /data (a Railway volume)
#     from the service's variables on the first start, then runs
#     `zecret maker --dir /data --relay 0.0.0.0:$PORT` as the unprivileged user zecret;
#   - the maker settings are the repository's testnet template
#     (crates/zecret-node/config/testnet-maker.json) with chains.birthday set to the
#     birthday of the maker's wallet.
#
# Local check (from the repository root):
#   docker build -f deploy/maker/railway/Dockerfile -t zecret-maker-railway .

ARG RUST_VERSION=1.97
ARG DEBIAN_RELEASE=bookworm

FROM rust:${RUST_VERSION}-slim-${DEBIAN_RELEASE} AS build
RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config libssl-dev ca-certificates protobuf-compiler \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY crates/ crates/
RUN cd crates \
 && cargo build --release --locked -p zecret-node --bin zecret \
 && install -m 0755 target/release/zecret /usr/local/bin/zecret

FROM debian:${DEBIAN_RELEASE}-slim
# The Zcash height where the maker's wallet starts to scan. The hosted maker uses the
# wallet that ran the first testnet swap (docs/TESTNET.md); it was created at 4614922.
ARG ZECRET_BIRTHDAY=4614922
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates tini jq \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --user-group --home-dir /data --shell /usr/sbin/nologin zecret
COPY --from=build /usr/local/bin/zecret /usr/local/bin/zecret
COPY crates/zecret-node/config/testnet-maker.json /tmp/testnet-maker.json
COPY deploy/maker/railway/start.sh /usr/local/bin/zecret-start
RUN mkdir -p /etc/zecret \
 && jq --argjson b "$ZECRET_BIRTHDAY" '.chains.birthday = $b' /tmp/testnet-maker.json > /etc/zecret/config.testnet.json \
 && rm /tmp/testnet-maker.json \
 && chmod 0755 /usr/local/bin/zecret-start \
 && command -v setpriv
# The start script runs as root only to prepare /data (Railway mounts volumes as root),
# then drops to uid 10001 with setpriv before it starts the maker.
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/zecret-start"]
