FROM rust:latest AS builder

WORKDIR /usr/src/app

COPY . .

ENV RUSTFLAGS="-Ctarget-cpu=native"
RUN cargo build --release

FROM debian:buster-slim

COPY --from=builder /usr/src/app/target/release/discord-embed-bot /usr/local/bin/discord_embed_bot

CMD ["discord_embed_bot"]