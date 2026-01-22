FROM rust:1.92-slim-bullseye AS builder

WORKDIR /usr/src/app

COPY Cargo.toml .
COPY Cargo.lock .

RUN mkdir src && echo "fn main() {println!(\"if you see this when RUNNING the bot, the build broke\")}" > src/main.rs

ENV RUSTFLAGS="-Ctarget-cpu=native"
RUN cargo fetch --locked
RUN cargo build --release

COPY . .

# Make sure Cargo sees the modifications
RUN find src -exec touch {} +

RUN cargo build --release

###############################################################################

FROM debian:bookworm-slim

RUN apt update && \
	apt install ffmpeg npm nodejs python3 python3-pip curl ca-certificates -y && \
	rm -rf /var/lib/apt/lists/*

RUN pip3 install --break-system-packages httpx aiofiles Pillow

WORKDIR /app

COPY package-lock.json .
COPY package.json .
RUN npm install

COPY --from=builder /usr/src/app/src/tiktok/tiktok.py /app/src/tiktok/tiktok.py
COPY --from=builder /usr/src/app/target/release/discord-embed-bot /app/discord-embed-bot

ENTRYPOINT ["./discord-embed-bot"]