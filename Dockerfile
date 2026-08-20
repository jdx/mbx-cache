FROM rust:1.97-bookworm AS builder
ARG MBX_CACHE_BUILD_REVISION=unknown
ENV MBX_CACHE_BUILD_REVISION=$MBX_CACHE_BUILD_REVISION
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY migrations ./migrations
COPY src ./src
RUN cargo build --locked --release

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=builder /src/target/release/mbx-cache /usr/local/bin/mbx-cache
ENV MBX_CACHE_DATA_DIR=/tmp/mbx-cache
EXPOSE 8080
USER nonroot:nonroot
ENTRYPOINT ["/usr/local/bin/mbx-cache"]
