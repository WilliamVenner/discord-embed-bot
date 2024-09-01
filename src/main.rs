use std::{
	borrow::Cow,
	path::{Path, PathBuf},
};

use config::ConfigDaemon;
use discord::DiscordBotDaemon;
use yt_dlp::YtDlpDaemon;

mod cmd;
mod config;
mod discord;
mod github;
mod logging;
mod yt_dlp;

pub struct App {
	pub discord_bot: DiscordBotDaemon,
}
impl App {
	pub async fn new(config_path: &Path, discord_bot_token: &str) -> Result<App, anyhow::Error> {
		let ctx = AppContext {
			config: ConfigDaemon::new(config_path).await?,
			yt_dlp: YtDlpDaemon::new().await?,
		};

		let discord_bot = DiscordBotDaemon::new(discord_bot_token, ctx).await?;

		Ok(Self { discord_bot })
	}

	pub async fn run(self) -> Result<(), anyhow::Error> {
		let ctrlc = tokio::signal::ctrl_c();

		tokio::select! {
			discord_bot = self.discord_bot => discord_bot?,

			_ = ctrlc => log::info!("Received Ctrl-C, shutting down..."),
		}

		Ok(())
	}
}

#[derive(Clone)]
pub struct AppContext {
	pub yt_dlp: YtDlpDaemon,
	pub config: ConfigDaemon,
}

#[tokio::main]
async fn main() {
	logging::DiscordLogger::init(
		pretty_env_logger::formatted_timed_builder()
			.filter_module("tracing", log::LevelFilter::Warn)
			.filter_module("serenity", log::LevelFilter::Warn)
			.filter_module("tokio", log::LevelFilter::Warn)
			.filter_level(log::LevelFilter::Info)
			.build(),
	);

	log::info!("Starting...");

	let mut discord_bot_token = None;
	let mut config_path = Cow::Borrowed(Path::new("config.json"));

	let mut args = std::env::args();
	while let Some(arg) = args.next() {
		if arg == "--discord-bot-token" {
			discord_bot_token = Some(args.next().expect("Expected a value for --discord-bot-token").into_boxed_str());
		} else if arg == "--config-path" {
			config_path = Cow::Owned(PathBuf::from(args.next().expect("Expected a value for --config-path")));
		} else if arg == "--discord-bot-token-path" {
			let discord_bot_token_path = PathBuf::from(args.next().expect("Expected a value for --discord-bot-token-path"));

			discord_bot_token = Some(
				std::fs::read_to_string(&discord_bot_token_path)
					.expect("Failed to read --discord-bot-token-path")
					.into_boxed_str(),
			);
		}
	}

	if discord_bot_token.is_none() && Path::new("discord_bot_token").is_file() {
		discord_bot_token = Some(
			std::fs::read_to_string("discord_bot_token")
				.expect("Failed to read discord_bot_token")
				.into_boxed_str(),
		);
	}

	if discord_bot_token.is_none() {
		if let Ok(token) = std::env::var("DISCORD_BOT_TOKEN") {
			discord_bot_token = Some(token.into_boxed_str());
		}
	}

	App::new(
		config_path.as_ref(),
		discord_bot_token
			.expect("Expected a --discord-bot-token or --discord-bot-token-path")
			.as_ref(),
	)
	.await
	.unwrap()
	.run()
	.await
	.unwrap();
}
