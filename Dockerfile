# Stage 1: Build
FROM rust:alpine AS builder
RUN apk add --no-cache musl-dev protobuf-dev openssl-dev openssl-libs-static pkgconfig ca-certificates
WORKDIR /app
COPY . .
# Build a statically linked binary
RUN OPENSSL_STATIC=1 cargo build --release --target x86_64-unknown-linux-musl

# Stage 2: Run
FROM scratch
# Copy the binary and CA certificates from the builder stage
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/mqttcasters /mqttcasters
ENTRYPOINT ["/mqttcasters"]
