# syntax=docker/dockerfile:1
# Server mode: API + background crawler + built frontend in one image.
#   docker build -t feed .
#   docker run -p 8080:8080 -v feed-data:/data feed

FROM node:22-trixie-slim AS web
WORKDIR /src/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci
COPY frontend/ ./
RUN npm run build

FROM rust:1-trixie AS server
WORKDIR /src/backend
COPY backend/ ./
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/backend/target \
    cargo build --release --locked && cp target/release/feed-server /feed-server

FROM debian:trixie-slim
# ca-certificates should track the distribution, not a pinned version.
# hadolint ignore=DL3008
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --home-dir /app feed \
 && mkdir -p /data \
 && chown feed /data
COPY --from=server /feed-server /usr/local/bin/feed-server
COPY --from=web /src/frontend/dist /app/web
COPY deploy/config.docker.toml /app/config.toml
# Mount your own config over /app/config.toml, or point FEED_CONFIG elsewhere.
# FEED_USER_AGENT overrides the User-Agent without a config file.
ENV FEED_CONFIG=/app/config.toml
USER feed
WORKDIR /app
VOLUME ["/data"]
EXPOSE 8080
CMD ["feed-server", "serve"]
