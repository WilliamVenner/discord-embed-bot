#!/bin/bash

sudo systemctl stop discord_embed_bot.service &&
docker stop discord_embed_bot || true &&
docker rm discord_embed_bot || true &&
docker build -t discord_embed_bot:latest . &&
docker create --name discord_embed_bot -v /etc/discord_embed_bot:/etc/discord_embed_bot -v /etc/discord_embed_bot:/etc/discord_embed_bot discord_embed_bot:latest &&
sudo cp discord_embed_bot.service /etc/systemd/system/ &&
sudo systemctl daemon-reload &&
sudo systemctl start discord_embed_bot.service