#!/bin/bash

docker build -t discord_embed_bot:latest . &&
sudo cp discord_embed_bot.service /etc/systemd/system/ &&
sudo systemctl enable discord_embed_bot.service &&
sudo systemctl start discord_embed_bot.service