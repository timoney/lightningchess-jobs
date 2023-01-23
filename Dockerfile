FROM rust:1.64.0 as builder
WORKDIR /app
COPY . .
RUN cargo install --profile release --path .

FROM debian:buster-slim as runner
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates wget gcc libssl-dev libc6-dev
COPY --from=builder /usr/local/cargo/bin/lightningchess-jobs /usr/local/bin/lightningchess-jobs
CMD ["lightningchess-jobs"]