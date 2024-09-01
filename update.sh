#!/bin/bash

sudo systemctl stop discord_embed_bot.service &&
docker stop discord_embed_bot &&
docker rm discord_embed_bot &&
docker build -t discord_embed_bot:latest . &&
sudo cp discord_embed_bot.service /etc/systemd/system/ &&
sudo systemctl daemon-reload &&
sudo systemctl start discord_embed_bot.service