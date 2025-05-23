use serde::{Deserialize, Serialize};
use serenity::all::{ChannelId, GuildId};
use std::{
	cell::{Cell, RefCell},
	path::Path,
	sync::{atomic::AtomicU16, Arc},
};
use tokio::{
	fs::{File, OpenOptions},
	io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
	sync::Mutex,
};

fn regex_macros(regex: &str) -> String {
	regex.replace("$URLCHAR", r#"[A-Za-z0-9\-._~:/?#\[\]@!$&'()*+,;=%]"#)
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
	pub link_regexes: Box<[LinkRegex]>,
	pub admin_guild: Option<AdminGuild>,
}
impl Default for Config {
	fn default() -> Self {
		Self {
			link_regexes: Box::new([]),
			admin_guild: None,
		}
	}
}

#[derive(Serialize, Deserialize, Debug)]
pub struct LinkRegex {
	pub regex: String,
	pub fixup: Option<String>,
	pub no_video: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AdminGuild {
	pub guild_id: GuildId,
	pub log_channel_id: ChannelId,
	pub config_channel_id: ChannelId,
}

pub struct CompiledConfig {
	pub link_regexes: Box<[CompiledLinkRegex]>,
	pub admin_guild: Option<AdminGuild>,
}
impl Default for CompiledConfig {
	fn default() -> Self {
		Self::try_from(&Config::default()).unwrap()
	}
}
impl TryFrom<&Config> for CompiledConfig {
	type Error = anyhow::Error;

	fn try_from(config: &Config) -> Result<Self, Self::Error> {
		Ok(Self {
			link_regexes: config
				.link_regexes
				.iter()
				.map(|regex| {
					Ok::<_, Self::Error>(CompiledLinkRegex {
						regex: regex::RegexBuilder::new(&regex_macros(&regex.regex)).case_insensitive(true).build()?,
						fixup: regex.fixup.as_deref().map(Into::into),
						no_video: regex.no_video.as_deref().map(Into::into),
					})
				})
				.collect::<Result<Vec<_>, _>>()?
				.into_boxed_slice(),

			admin_guild: config.admin_guild.clone(),
		})
	}
}

pub struct CompiledLinkRegex {
	pub regex: regex::Regex,
	pub fixup: Option<Box<str>>,
	pub no_video: Option<Box<str>>,
}

#[derive(Clone)]
pub struct ConfigDaemon(Arc<ConfigDaemonInner>);
impl ConfigDaemon {
	pub async fn new(config_path: &Path) -> Result<Self, anyhow::Error> {
		let mut file = OpenOptions::new()
			.truncate(false)
			.write(true)
			.read(true)
			.append(false)
			.create(true)
			.open(config_path)
			.await?;

		let size = file.metadata().await?.len();

		let config = if size == 0 {
			Config::default()
		} else {
			let mut config = Vec::with_capacity(size as usize);
			file.read_to_end(&mut config).await?;
			serde_json::from_slice(&config)?
		};

		let compiled_config = CompiledConfig::try_from(&config)?;

		file.set_len(0).await?;
		file.seek(std::io::SeekFrom::Start(0)).await?;
		file.write_all(serde_json::to_string_pretty(&config)?.as_bytes()).await?;

		Ok(Self(Arc::new(ConfigDaemonInner {
			edit_count: AtomicU16::new(0),
			store: Mutex::new(ConfigStore {
				file,
				config: SignedConfig {
					signature: 0,
					config: Arc::new(compiled_config),
				},
			}),
		})))
	}

	pub async fn dump(&self) -> Result<String, anyhow::Error> {
		let mut store = self.0.store.lock().await;

		let mut dump = String::new();

		store.file.seek(std::io::SeekFrom::Start(0)).await?;
		store.file.read_to_string(&mut dump).await?;

		Ok(dump)
	}

	pub async fn edit(&self, new: &str) -> Result<(), anyhow::Error> {
		let config = serde_json::from_str(new)?;
		let compiled_config = CompiledConfig::try_from(&config)?;

		{
			let mut store = self.0.store.lock().await;
			let edit_count = self.0.edit_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

			store.file.set_len(0).await?;
			store.file.seek(std::io::SeekFrom::Start(0)).await?;
			store.file.write_all(serde_json::to_string_pretty(&config)?.as_bytes()).await?;

			store.config = SignedConfig {
				signature: edit_count + 1,
				config: Arc::new(compiled_config),
			};
		}

		Ok(())
	}

	pub async fn get(&self) -> Arc<CompiledConfig> {
		struct ThreadLocalConfigCache {
			config: RefCell<Arc<CompiledConfig>>,
			edit_count: Cell<u16>,
		}

		thread_local! {
			static CONFIG_CACHE: ThreadLocalConfigCache = ThreadLocalConfigCache {
				config: RefCell::new(Arc::new(CompiledConfig::default())),
				edit_count: Cell::new(u16::MAX),
			};
		}

		if let Some(cached) = CONFIG_CACHE.with(|cache| {
			if cache.edit_count.get() == self.0.edit_count.load(std::sync::atomic::Ordering::Acquire) {
				Some(cache.config.borrow().clone())
			} else {
				None
			}
		}) {
			return cached;
		}

		loop {
			let SignedConfig { signature, config } = self.0.store.lock().await.config.clone();

			CONFIG_CACHE.with(|cache| {
				*cache.config.borrow_mut() = config.clone();
				cache.edit_count.set(signature);
			});

			if self.0.edit_count.load(std::sync::atomic::Ordering::Acquire) == signature {
				break config;
			}
		}
	}
}

#[derive(Clone)]
struct SignedConfig {
	signature: u16,
	config: Arc<CompiledConfig>,
}

struct ConfigStore {
	config: SignedConfig,
	file: File,
}

struct ConfigDaemonInner {
	store: Mutex<ConfigStore>,
	edit_count: AtomicU16,
}

#[test]
fn default_config_compiles() {
	let _ = CompiledConfig::default();
}
