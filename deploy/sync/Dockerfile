FROM rust:1.92-bookworm AS build
WORKDIR /source
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --locked --release -p qf-sync-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 10001 --no-create-home --shell /usr/sbin/nologin queue \
    && mkdir /data /backups \
    && chown queue:queue /data /backups && chmod 0700 /data /backups
COPY --from=build /source/target/release/qf-sync-server /usr/local/bin/qf-sync-server
USER 10001:10001
EXPOSE 8787
ENTRYPOINT ["qf-sync-server"]
CMD ["serve", "/data/queue.db", "0.0.0.0:8787"]
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl --fail --silent --max-time 4 http://127.0.0.1:8787/health || exit 1
