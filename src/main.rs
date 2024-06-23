use std::sync::Arc;

use yt_dlp::YtDlpDaemon;

mod github;
mod util;
mod yt_dlp;

pub struct App {
	pub yt_dlp: Arc<YtDlpDaemon>,
}
impl App {
	pub async fn new() -> Result<App, anyhow::Error> {
		if std::env::var_os("RUST_LOG").is_none() {
			std::env::set_var("RUST_LOG", "info");
		}

		pretty_env_logger::init_timed();

		let yt_dlp = YtDlpDaemon::new().await?;

		Ok(Self { yt_dlp })
	}

	pub async fn run(self) -> Result<(), anyhow::Error> {
		Ok(())
	}
}

#[tokio::main]
async fn main() {
	App::new().await.unwrap().run().await.unwrap();
}
