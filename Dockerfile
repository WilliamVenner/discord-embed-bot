FROM rust:latest AS builder

WORKDIR /usr/src/app

RUN apt update
RUN apt install ffmpeg -y

COPY . .

RUN cargo fetch

COPY . .

ENV RUSTFLAGS="-Ctarget-cpu=native"
RUN cargo build --release

ENTRYPOINT ["./target/release/discord-embed-bot"]