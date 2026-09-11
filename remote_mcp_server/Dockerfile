# OCI index digests verified against registry-1.docker.io on 2026-09-11.
FROM rust:1.97.1-bookworm@sha256:0e2bcaef56d041a486784e54104a81aebe0da44bd03019bd70bc0401e42e4a97 AS builder

WORKDIR /build
COPY . .
RUN cargo build --locked --release -p cdr-mcp-server --bin cdr-mcp-server

FROM debian:bookworm-slim@sha256:88200866dfff7ea7f5cbcb6ec7c8a701889efe6fe859fe64d6990e4b07ea4171

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 simdorei \
    && useradd --uid 10001 --gid 10001 --no-create-home \
        --home-dir /nonexistent --shell /usr/sbin/nologin simdorei \
    && install -d --owner=10001 --group=10001 --mode=0700 /data

COPY --from=builder /build/target/release/cdr-mcp-server /usr/local/bin/cdr-mcp-server

USER 10001:10001
EXPOSE 8030
ENTRYPOINT ["/usr/local/bin/cdr-mcp-server"]
