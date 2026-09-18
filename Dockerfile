FROM rust:alpine AS builder

WORKDIR /src

COPY . .

RUN mkdir -m 1777 /runtime-tmp \
    && apk add --no-cache ca-certificates \
    && cargo build --release --locked

FROM scratch

COPY --from=builder /runtime-tmp /tmp
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=builder /src/target/release/nowhere /nowhere

ENTRYPOINT ["/nowhere"]
