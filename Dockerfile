ARG RUST_VERSION=1.88.0

FROM rust:${RUST_VERSION}-alpine AS builder
WORKDIR /app

RUN apk add --no-cache musl-dev openssl-dev openssl-libs-static
COPY . .
RUN \
  --mount=type=cache,target=/app/target/ \
  --mount=type=cache,target=/usr/local/cargo/registry/ \
  cargo build --release && \
    cp ./target/release/every-frame /

FROM alpine:3 AS final
WORKDIR /app
RUN addgroup -S myuser && adduser -S myuser -G myuser
COPY --from=builder /every-frame .
USER myuser
CMD ["./every-frame"]