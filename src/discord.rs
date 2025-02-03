use crate::{cmd, config::CompiledConfig, logging, AppContext};
use serenity::{
	all::{
		CreateAllowedMentions, CreateAttachment, CreateEmbed, CreateInteractionResponse, CreateInteractionResponseMessage, CreateMessage,
		EditMessage, Interaction, Message, MessageUpdateEvent,
	},
	async_trait,
	futures::StreamExt,
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
		| GatewayIntents::DIRECT_MESSAGES
		| GatewayIntents::DIRECT_MESSAGE_REACTIONS
		| GatewayIntents::DIRECT_MESSAGE_TYPING
}

#[derive(Clone)]
struct DiscordBot {
	app_ctx: AppContext,
}
impl DiscordBot {
	async fn generic_message(&self, ctx: Context, mut msg: Message, config: Arc<CompiledConfig>) {
		// Ignore NotSoBot .dl commands
		if msg.content.trim().starts_with(".dl ") {
			return;
		}

		let mut download_urls = config
			.link_regexes
			.iter()
			.flat_map(|regex| regex.find_iter(&msg.content))
			.map(|match_| match_.as_str());

		let Some(download_url) = download_urls.next() else {
			return;
		};

		// Reject multiple URLs
		if download_urls.next().is_some() {
			return;
		}

		let typing = msg.channel_id.start_typing(&ctx.http);

		let mut replace_embed = {
			match msg.embeds.len() {
				0 => {
					// Wait for message to have an embed, if any
					let mut message_updates = serenity::collector::collect(&ctx.shard, move |ev| match ev {
						serenity::all::Event::MessageUpdate(MessageUpdateEvent {
							id, embeds: Some(embeds), ..
						}) if *id == msg.id => Some(if embeds.len() == 1 { Some(embeds[0].clone()) } else { None }),
						_ => None,
					});
					match tokio::time::timeout(Duration::from_millis(2000), message_updates.next()).await {
						Ok(Some(Some(embed))) => Some(embed),
						_ => None,
					}
				}
				1 => Some(msg.embeds[0].clone()),
				_ => None,
			}
		};

		let media = match self.app_ctx.yt_dlp.download(download_url).await {
			Ok(media) => media,

			Err(err) => {
				log::error!("Failed to download {download_url} ({err})");
				return;
			}
		};

		let file = match CreateAttachment::path(&media.path).await {
			Ok(file) => file,
			Err(err) => {
				log::error!("Failed to create attachment for {download_url} ({err})");
				msg.react(&ctx, '❌').await.ok();
				return;
			}
		};

		let mut reply = CreateMessage::new()
			.reference_message(&msg)
			.add_file(file)
			.allowed_mentions(CreateAllowedMentions::new());

		if let Some(embed) = &mut replace_embed {
			embed.image = None;
			embed.video = None;
			embed.thumbnail = None;
			embed.provider = None;
			reply = reply.add_embed(CreateEmbed::from(embed.clone()));
		}

		let result = msg.channel_id.send_message(&ctx, reply.clone()).await.map(|_| ());

		drop(typing);

		if let Err(err) = &result {
			if err.to_string().as_str() == "Request entity too large" {
				if let Some(raw_url) = media.url.as_deref() {
					let reply = CreateMessage::new().content(format!("-# File was too large to upload\n{raw_url}"));

					if let Err(err) = msg.channel_id.send_message(&ctx, reply).await {
						log::error!("Failed to send secondary \"file was too large\" message for {download_url} ({err} {err:?})");
						msg.react(&ctx, '❌').await.ok();
					} else {
						return;
					}
				} else {
					log::error!("Failed to send secondary \"file was too large\" message for {download_url} (no raw URL found)");
					msg.react(&ctx, '❌').await.ok();
				}
			}
		}

		match result {
			Err(serenity::Error::Http(serenity::http::HttpError::UnsuccessfulRequest(serenity::http::ErrorResponse {
				status_code: serenity::http::StatusCode::PAYLOAD_TOO_LARGE,
				..
			}))) => {
				// TODO try to compress the video further
				msg.react(&ctx, '🫃').await.ok();
			}

			Err(err) => {
				log::error!("Failed to send {download_url} ({err} {err:?})");
				msg.react(&ctx, '❌').await.ok();
			}

			Ok(_) => {
				if replace_embed.is_some() {
					msg.edit(ctx, EditMessage::new().suppress_embeds(true)).await.ok();
				}
			}
		}
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
	async fn ready(&self, ctx: Context, ready: serenity::all::Ready) {
		log::info!("Discord bot connected as {}", ready.user.name);
		log::info!(
			"Invite link: https://discord.com/oauth2/authorize?client_id={}&permissions=274877966400&integration_type=0&scope=bot",
			ready.user.id
		);
		log::info!("Member of {} guilds", ready.guilds.len());

		cmd::register(&ctx).await.expect("Failed to register /download command");

		let config = self.app_ctx.config.get().await;

		if let Some(admin_guild) = &config.admin_guild {
			logging::connect_discord(admin_guild.log_channel_id, ctx.clone()).await;
		}
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

	async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
		if let Interaction::Command(command) = interaction {
			if command.data.name.as_str() == "download" {
				if let Err(err) = cmd::run(&self.app_ctx, &ctx, &command, &command.data.options()).await {
					log::error!("Failed to run /download command: {err}");

					command
						.create_response(
							ctx,
							CreateInteractionResponse::Message(
								CreateInteractionResponseMessage::new().ephemeral(true).content("Internal error occurred"),
							),
						)
						.await
						.ok();
				}
			}
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
