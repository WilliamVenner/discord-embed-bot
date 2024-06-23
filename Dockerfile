FROM rust:latest AS builder

WORKDIR /usr/src/app

COPY . .

ENV RUSTFLAGS="-Ctarget-cpu=native"
RUN cargo build --release

CMD ["./target/release/discord-embed-bot"]