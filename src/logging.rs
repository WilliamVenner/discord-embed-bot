use serenity::all::{ChannelId, Colour, Context, CreateEmbed, CreateMessage, Timestamp};
use std::sync::OnceLock;

type AppLogger = pretty_env_logger::env_logger::Logger;

static LOGGER: OnceLock<DiscordLogger> = OnceLock::new();

struct DiscordLoggerContext {
	ctx: Context,
	channel_id: ChannelId,
	rt: tokio::runtime::Handle,
}

pub struct DiscordLogger {
	logger: AppLogger,
	ctx: OnceLock<DiscordLoggerContext>,
}
impl DiscordLogger {
	pub fn init(logger: AppLogger) {
		log::set_max_level(log::LevelFilter::Info);
		log::set_logger(LOGGER.get_or_init(|| Self {
			logger,
			ctx: OnceLock::new(),
		}))
		.unwrap();
	}
}
impl log::Log for DiscordLogger {
	fn enabled(&self, metadata: &log::Metadata) -> bool {
		self.logger.enabled(metadata)
	}

	fn log(&self, record: &log::Record) {
		self.logger.log(record);

		if let Some(DiscordLoggerContext { rt, ctx, channel_id }) = self.ctx.get() {
			let msg = CreateMessage::new().add_embed({
				let mut embed = CreateEmbed::new()
					.timestamp(Timestamp::now())
					.description(format!("```\n{}\n```", record.args()))
					.color(match record.level() {
						log::Level::Info => Colour::DARK_GREEN,
						log::Level::Warn => Colour::DARK_GOLD,
						log::Level::Error => Colour::DARK_RED,
						log::Level::Debug | log::Level::Trace => return,
					});

				if let Some(module_path) = record.module_path() {
					embed = embed.title(module_path);
				}

				embed
			});

			let channel_id = *channel_id;
			let ctx = ctx.clone();
			rt.spawn(async move {
				if let Err(err) = ctx.http.send_message(channel_id, Vec::new(), &msg).await {
					eprintln!("Failed to send log message to Discord: {err} {err:?}");
				}
			});
		}
	}

	fn flush(&self) {
		self.logger.flush();
	}
}

pub async fn connect_discord(channel_id: ChannelId, ctx: Context) {
	let logger = match LOGGER.get() {
		Some(logger) => logger,
		None if cfg!(debug_assertions) => unreachable!("Discord logger not initialized"),
		None => return,
	};

	let rt = tokio::runtime::Handle::current();

	logger.ctx.get_or_init(|| DiscordLoggerContext { rt, ctx, channel_id });

	log::info!("Connected to Discord logging channel");
}
