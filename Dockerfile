####################################################################################################
## Builder
####################################################################################################
FROM alpine:latest AS builder

RUN apk --no-cache add rustup cargo g++ openssl openssl-dev git
RUN rustup-init -y
ENV PATH=/root/.cargo/bin:$PATH
ENV OPENSSL_STATIC=yes \
    PKG_CONFIG_ALLOW_CROSS=true \
    PKG_CONFIG_ALL_STATIC=true \
    RUSTFLAGS="-C target-feature=-crt-static"

RUN adduser \
    --disabled-password \
    --gecos "" \
    --home "/nonexistent" \
    --shell "/sbin/nologin" \
    --no-create-home \
    --uid "1000" \
    "pview"

WORKDIR /work
COPY . .

RUN --mount=type=ssh \
    --mount=type=cache,target=/root/.cargo/registry \
    --mount=type=cache,target=/root/.cargo/git \
    --mount=type=cache,target=/work/target \
    cargo install --bin pview --path .

####################################################################################################
## Final image
####################################################################################################
FROM scratch

# Import from builder.
COPY --from=builder /etc/passwd /etc/passwd
COPY --from=builder /etc/group /etc/group

WORKDIR /app

COPY --from=builder /lib/ld-musl*.so* /lib/libssl*.so* /lib/libcrypto*.so* /usr/lib/libgcc*.so* /lib/
COPY --from=builder /root/.cargo/bin/pview ./

USER pview:pview
LABEL org.opencontainers.image.source="https://github.com/wez/pview"
ENV \
  RUST_BACKTRACE=full \
  PATH=/app:$PATH

CMD ["/app/pview", "serve-mqtt"]

