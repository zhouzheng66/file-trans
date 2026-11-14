FROM rust:1-slim-bookworm AS build
WORKDIR /src
RUN apt-get update && apt-get install -y --no-install-recommends build-essential pkg-config ca-certificates libdbus-1-dev && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY services ./services
COPY apps/desktop/src-tauri/Cargo.toml ./apps/desktop/src-tauri/Cargo.toml
# 保持 workspace 与锁文件一致，桌面目标只提供元数据入口，不参与服务编译。
RUN mkdir -p apps/desktop/src-tauri/src && printf 'fn main() {}\n' > apps/desktop/src-tauri/src/main.rs
RUN cargo build --release --locked -p file-trans-relay-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libdbus-1-3 && rm -rf /var/lib/apt/lists/*
RUN useradd --system --uid 10001 --create-home relay
COPY --from=build /src/target/release/file-trans-relay-server /usr/local/bin/file-trans-relay-server
USER relay
ENV RELAY_LISTEN=0.0.0.0:8787
EXPOSE 8787
ENTRYPOINT ["file-trans-relay-server"]
