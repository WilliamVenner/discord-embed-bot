use crate::{config::CompiledConfig, AppContext};
use serenity::{
	all::{CreateAllowedMentions, CreateAttachment, CreateMessage, Message},
	async_trait,
	prelude::*,
	FutureExt,
};
use std::{future::Future, sync::Arc, time::Duration};

fn discord_bot_permissions() -> GatewayIntents {
	GatewayIntents::GUILD_MESSAGES
		| GatewayIntents::MESSAGE_CONTENT
		| GatewayIntents::GUILD_MESSAGE_REACTIONS
		| GatewayIntents::GUILD_MESSAGE_TYPING
		| GatewayIntents::GUILD_EMOJIS_AND_STICKERS
		| GatewayIntents::GUILD_MESSAGES
}

#[derive(Clone)]
struct DiscordBot {
	app_ctx: AppContext,
}
impl DiscordBot {
	async fn generic_message(&self, ctx: Context, msg: Message, config: Arc<CompiledConfig>) {
		let mut download_urls = config.link_regexes.iter().filter_map(|regex| {
			let captures = regex.captures(&msg.content)?;
			Some(captures.get(1).unwrap_or_else(|| captures.get(0).unwrap()).as_str())
		});

		let Some(download_url) = download_urls.next() else {
			return;
		};

		if download_urls.next().is_some() {
			return;
		}

		let typing = msg.channel_id.start_typing(&ctx.http);

		let media = match self.app_ctx.yt_dlp.download(download_url).await {
			Ok(media) => media,
			Err(err) => {
				log::error!("Failed to download {download_url} ({err})");
				msg.react(ctx, '❌').await.ok();
				return;
			}
		};

		let file = match CreateAttachment::path(&media.path).await {
			Ok(file) => file,
			Err(err) => {
				log::error!("Failed to create attachment for {download_url} ({err})");
				msg.react(ctx, '❌').await.ok();
				return;
			}
		};

		if let Err(err) = msg
			.channel_id
			.send_message(
				&ctx,
				CreateMessage::new()
					.reference_message(&msg)
					.add_file(file)
					.allowed_mentions(CreateAllowedMentions::new()),
			)
			.await
		{
			log::error!("Failed to send {download_url} ({err})");
			msg.react(ctx, '❌').await.ok();
		}

		drop(typing);
	}

	async fn admin_config_message(&self, ctx: Context, msg: Message, _config: Arc<CompiledConfig>) {
		let mut content = msg.content.as_str();

		content = content.trim();

		content = content
			.strip_prefix("```json\n")
			.or_else(|| content.strip_prefix("```\n"))
			.and_then(|content| content.strip_suffix("\n```"))
			.unwrap_or(content);

		match self.app_ctx.config.edit(content).await {
			Ok(_) => {
				msg.react(ctx, '✅').await.ok();
			}

			Err(err) => {
				msg.reply(ctx, format!("ERROR: {err}")).await.ok();
			}
		}
	}

	fn is_admin_config_message(msg: &Message, config: &CompiledConfig) -> bool {
		config.admin_guild.as_ref().is_some_and(|admin_guild| {
			msg.guild_id.is_some_and(|guild_id| guild_id == admin_guild.guild_id) && msg.channel_id == admin_guild.config_channel_id
		})
	}
}

#[async_trait]
impl EventHandler for DiscordBot {
	async fn ready(&self, _ctx: Context, ready: serenity::all::Ready) {
		log::info!("Discord bot connected as {}", ready.user.name);
		log::info!("Invite link: https://discord.com/oauth2/authorize?client_id={}", ready.user.id);
		log::info!("Member of {} guilds", ready.guilds.len());
	}

	async fn message(&self, ctx: Context, msg: Message) {
		if msg.author.bot {
			return;
		}

		let config = self.app_ctx.config.get().await;

		if Self::is_admin_config_message(&msg, &config) {
			self.admin_config_message(ctx, msg, config).await;
		} else {
			self.generic_message(ctx, msg, config).await;
		}
	}
}

pub struct DiscordBotDaemon {
	task: tokio::task::JoinHandle<()>,
}
impl DiscordBotDaemon {
	pub async fn new(discord_bot_token: &str, app_ctx: AppContext) -> Result<Self, anyhow::Error> {
		let discord_bot_token = discord_bot_token.to_owned();

		let task = tokio::spawn(async move {
			let bot = DiscordBot { app_ctx };
			let mut first_run = true;
			loop {
				let res = async {
					let mut client = Client::builder(&discord_bot_token, discord_bot_permissions())
						.event_handler(bot.clone())
						.await?;

					client.start().await
				}
				.await;

				if let Err(err) = res {
					if first_run {
						panic!("Discord bot error: {}", err);
					} else {
						log::error!("Discord bot error: {}", err);
					}
				}

				first_run = false;

				tokio::time::sleep(Duration::from_secs(5)).await;
			}
		});

		Ok(Self { task })
	}
}
impl Future for DiscordBotDaemon {
	type Output = Result<(), tokio::task::JoinError>;

	fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
		self.get_mut().task.poll_unpin(cx)
	}
}
impl Drop for DiscordBotDaemon {
	fn drop(&mut self) {
		self.task.abort();
	}
}
