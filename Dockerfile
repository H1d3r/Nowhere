FROM rust:alpine AS builder

WORKDIR /src

COPY . .

RUN apk add --no-cache ca-certificates \
    && cargo build --release --locked

FROM scratch

COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=builder /src/target/release/nowhere /nowhere

ENTRYPOINT ["/nowhere"]
