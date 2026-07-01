FROM rust:1-alpine AS builder

RUN apk add --no-cache build-base cmake pkgconfig ca-certificates

ENV OPUS_BUNDLED=1

WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
COPY src ./src
RUN cargo build --release

FROM alpine:3.20

RUN apk add --no-cache ca-certificates ffmpeg

WORKDIR /app
COPY --from=builder /app/target/release/nekostream /usr/local/bin/nekostream

EXPOSE 3030
CMD ["nekostream"]
