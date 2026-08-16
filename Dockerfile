FROM rust:1.97-bookworm AS builder
ARG MISE_CACHE_BUILD_REVISION=unknown
ENV MISE_CACHE_BUILD_REVISION=$MISE_CACHE_BUILD_REVISION
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY migrations ./migrations
COPY src ./src
RUN cargo build --locked --release

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=builder /src/target/release/mise-cache /usr/local/bin/mise-cache
ENV MISE_CACHE_DATA_DIR=/tmp/mise-cache
EXPOSE 8080
USER nonroot:nonroot
ENTRYPOINT ["/usr/local/bin/mise-cache"]
