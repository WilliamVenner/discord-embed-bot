```bash
docker build -t discord_embed_bot:latest .
# docker stop discord_embed_bot
# docker rm discord_embed_bot
docker create --name discord_embed_bot --restart unless-stopped discord_embed_bot:latest
sudo cp discord_embed_bot.service /etc/systemd/system/
sudo systemctl enable discord_embed_bot.service
sudo systemctl start discord_embed_bot.service
```
