FROM rust:latest AS builder

WORKDIR /usr/src/app

RUN apt update
RUN apt install ffmpeg -y
RUN apt install npm nodejs -y

COPY package-lock.json .
COPY package.json .
RUN npm install

COPY Cargo.toml .
COPY Cargo.lock .

RUN mkdir src && echo "fn main() {println!(\"if you see this, the build broke\")}" > src/main.rs

ENV RUSTFLAGS="-Ctarget-cpu=native"
RUN cargo build --release

COPY . .

RUN cargo build --release

ENTRYPOINT ["./target/release/discord-embed-bot"]