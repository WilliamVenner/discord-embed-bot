FROM rust:latest AS builder

WORKDIR /usr/src/app

RUN apt update
RUN apt install ffmpeg -y

RUN cargo fetch

COPY . .

ENV RUSTFLAGS="-Ctarget-cpu=native"
RUN cargo build --release

CMD ["./target/release/discord-embed-bot"]