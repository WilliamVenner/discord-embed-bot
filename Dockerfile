FROM rust:latest AS builder

WORKDIR /usr/src/app

RUN apt update
RUN apt install ffmpeg -y
RUN apt install npm nodejs -y

COPY package-lock.json .
COPY package.json .
COPY Cargo.toml .
COPY Cargo.lock .

RUN cargo fetch
RUN npm install

COPY . .

ENV RUSTFLAGS="-Ctarget-cpu=native"
RUN cargo build --release

ENTRYPOINT ["./target/release/discord-embed-bot"]