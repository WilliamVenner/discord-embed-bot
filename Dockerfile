FROM rust:latest AS builder

WORKDIR /usr/src/app

COPY . .

RUN apt update
RUN apt install ffmpeg -y

ENV RUSTFLAGS="-Ctarget-cpu=native"
RUN cargo build --release

CMD ["./target/release/discord-embed-bot"]