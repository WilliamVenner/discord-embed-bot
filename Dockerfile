FROM rust:latest AS builder

WORKDIR /usr/src/app

RUN apt update
RUN apt install ffmpeg -y
RUN apt install npm nodejs -y

COPY . .

RUN cargo fetch
RUN npm install

COPY . .

ENV RUSTFLAGS="-Ctarget-cpu=native"
RUN cargo build --release

ENTRYPOINT ["./target/release/discord-embed-bot"]