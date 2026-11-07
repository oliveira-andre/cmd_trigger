# The CmdTrigger server. Build from the repository root:
#   docker build -f deploy/Dockerfile -t cmdtrigger-server .
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p cmdtrigger-server

FROM debian:bookworm-slim
RUN useradd --system --uid 10001 --home-dir /data cmdtrigger \
    && mkdir -p /data && chown cmdtrigger /data
COPY --from=build /src/target/release/cmdtrigger-server /usr/local/bin/cmdtrigger-server
USER cmdtrigger
ENV CMDTRIGGER_DB=/data/cmdtrigger.db \
    CMDTRIGGER_ADDR=0.0.0.0:8080
VOLUME /data
EXPOSE 8080
CMD ["cmdtrigger-server"]
