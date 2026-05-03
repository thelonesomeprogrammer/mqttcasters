# Stage 1: Build
FROM rust:alpine AS builder
RUN apk add --no-cache musl-dev protobuf-dev
WORKDIR /app
COPY . .
# Build a statically linked binary
RUN cargo build --release --target x86_64-unknown-linux-musl

# Stage 2: Run
FROM scratch
# Copy the binary from the builder stage
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/mqttcasters /mqttcasters
ENTRYPOINT ["/mqttcasters"]
