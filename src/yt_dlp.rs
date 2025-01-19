use crate::{github, tiktok, USER_AGENT};
use anyhow::Context;
use std::{
	borrow::Cow,
	path::{Path, PathBuf},
	sync::Arc,
	time::{Duration, Instant},
};
use tokio::{
	fs::File,
	process::Command,
	sync::{Mutex, RwLock},
};

const YT_DLP_EXE: &str = {
	#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
	{
		"yt-dlp.exe"
	}
	#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
	{
		"yt-dlp_linux"
	}
	#[cfg(target_os = "macos")]
	{
		"yt-dlp_macos"
	}
};

const YT_DLP_ARGS: &[&str] = &[
	"-f",
	"bestvideo[filesize<22MB]+bestaudio[filesize<3MB]/best/bestvideo+bestaudio",
	"-S",
	"vcodec:h264",
	"--merge-output-format",
	"mp4",
	"--ignore-config",
	"--verbose",
	"--no-playlist",
	"--no-warnings",
	"-o",
];

const YT_DLP_UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(30 * 60); // 30 mins

#[derive(Debug)]
struct YtDlpRelease {
	tag_name: Box<str>,
	browser_download_url: Box<str>,
	size: u64,
}
impl YtDlpRelease {
	async fn latest() -> Result<Self, anyhow::Error> {
		log::info!("Grabbing latest yt-dlp release...");

		let (tag_name, (browser_download_url, size)) = github::Releases::get("yt-dlp/yt-dlp", Duration::from_secs(7))
			.await?
			.0
			.into_iter()
			.find_map(|release| {
				if !release.draft && !release.prerelease {
					Some((
						release.tag_name,
						release.assets.into_iter().find_map(|asset| {
							if asset.name.as_ref() == YT_DLP_EXE {
								Some((asset.browser_download_url, asset.size))
							} else {
								None
							}
						})?,
					))
				} else {
					None
				}
			})
			.ok_or_else(|| anyhow::anyhow!("No release found"))?;

		log::info!("Latest yt-dlp release: {}", tag_name);

		Ok(YtDlpRelease {
			tag_name,
			browser_download_url,
			size,
		})
	}
}

pub struct YtDlp {
	tag_name: Box<str>,
	exe_path: Box<Path>,
}
impl YtDlp {
	pub async fn new() -> Result<Self, anyhow::Error> {
		let release = YtDlpRelease::latest().await?;
		Self::download_release(release).await
	}

	async fn download_release(release: YtDlpRelease) -> Result<Self, anyhow::Error> {
		log::info!("Downloading yt-dlp release {}", release.tag_name);

		let YtDlpRelease {
			tag_name,
			browser_download_url,
			size,
		} = release;

		let fs_tag_name = tag_name
			.chars()
			.map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
			.collect::<String>();

		let exe_path = Path::new("yt_dlp_exe")
			.join({
				let mut yt_dlp_exe = PathBuf::from(YT_DLP_EXE);

				let ext = yt_dlp_exe.extension().map(ToOwned::to_owned);

				yt_dlp_exe.set_file_name(format!("yt_dlp_{fs_tag_name}"));

				if let Some(ext) = ext {
					yt_dlp_exe.set_extension(ext);
				}

				yt_dlp_exe
			})
			.into_boxed_path();

		log::info!(
			"Checking if yt-dlp release {} has already been downloaded to {}",
			tag_name,
			exe_path.display()
		);

		if exe_path.metadata().is_ok_and(|m| m.len() == size) {
			log::info!("yt-dlp release {} already downloaded", tag_name);

			return Ok(Self { tag_name, exe_path });
		}

		log::info!("Downloading yt-dlp release {}", tag_name);

		if Path::new("yt_dlp_exe").is_dir() {
			tokio::fs::remove_dir_all("yt_dlp_exe").await?;
		}

		tokio::fs::create_dir_all("yt_dlp_exe").await?;

		let mut exe = File::create(exe_path.as_ref()).await?;

		tokio::io::copy(&mut reqwest::get(browser_download_url.as_ref()).await?.bytes().await?.as_ref(), &mut exe).await?;

		log::info!("Downloaded yt-dlp release {}", tag_name);

		if cfg!(target_os = "linux") {
			let output = tokio::process::Command::new("chmod").arg("+x").arg(exe_path.as_ref()).output().await?;

			if !output.status.success() {
				return Err(anyhow::anyhow!("Failed to chmod yt-dlp (status {})", output.status));
			}
		}

		Ok(Self { tag_name, exe_path })
	}

	pub async fn download<'a>(&self, url: &str, out_path: &'a Path) -> Result<DownloadedMedia, anyhow::Error> {
		log::info!("Downloading {url} to {}", out_path.display());

		let output = Command::new(self.exe_path.as_ref())
			.args(YT_DLP_ARGS)
			.arg(out_path)
			.arg(url)
			.output()
			.await?;

		log::info!("Downloaded {url} to {}", out_path.display());

		if cfg!(debug_assertions) {
			println!("===== EXIT CODE {} =====", output.status);
			println!("===== STDOUT =====\n{}\n", String::from_utf8_lossy(&output.stdout));
			println!("===== STDERR =====\n{}", String::from_utf8_lossy(&output.stderr));
		}

		if output.status.success() {
			if out_path.exists() {
				let mut out_path = Cow::Borrowed(out_path);

				match (cfg!(debug_assertions), self.video_integrity_check(out_path.as_ref()).await) {
					(true, Err(err)) => panic!("Video integrity check failed: {err}"),
					(false, Err(err)) => log::error!("Video integrity check failed: {err}"),

					(_, Ok(false)) => {
						log::info!("Video appears to be corrupt, re-encoding...");

						match self.reencode_video(out_path.as_ref()).await {
							Ok(new_out_path) => {
								out_path = Cow::Owned(new_out_path);
							}

							Err(err) => log::error!("Failed to re-encode video: {err}"),
						}
					}

					(_, Ok(true)) => {}
				}

				let url = (|| {
					let stdout = std::str::from_utf8(&output.stdout).ok()?;
					let dump = serde_json::from_str::<YtDlpJsonDump>(stdout).ok()?;

					if dump.requested_downloads.len() == 1 {
						Some(dump.requested_downloads[0].url.as_str().into())
					} else {
						Some(dump.url.into_boxed_str())
					}
				})();

				Ok(DownloadedMedia { path: out_path.into(), url })
			} else {
				Err(anyhow::anyhow!("yt-dlp did not create the file"))
			}
		} else {
			Err(anyhow::anyhow!(
				"Exit status: {}\n\n=========== stderr ===========\n{}\n\n=========== stdout ===========\n{}",
				output.status,
				String::from_utf8_lossy(&output.stderr),
				String::from_utf8_lossy(&output.stdout)
			))
		}
	}

	async fn video_integrity_check(&self, path: &Path) -> Result<bool, std::io::Error> {
		let output = Command::new(if cfg!(windows) { "ffprobe.exe" } else { "ffprobe" })
			.arg(path)
			.output()
			.await?;

		let stdout = String::from_utf8_lossy(&output.stdout);
		let stdout = stdout.as_ref();

		let stderr = String::from_utf8_lossy(&output.stderr);
		let stderr = stderr.as_ref();

		if stderr.contains("Packet corrupt") || stdout.contains("Packet corrupt") {
			return Ok(false);
		}

		Ok(true)
	}

	async fn reencode_video(&self, path: &Path) -> Result<PathBuf, std::io::Error> {
		let reencoded_path = path.with_file_name(format!("{}_reencoded.mp4", path.file_stem().unwrap().to_string_lossy()));

		let output = Command::new(if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" })
			.arg("-i")
			.arg(path)
			.args(["-vcodec", "libx264", "-acodec", "aac"])
			.arg(&reencoded_path)
			.output()
			.await?;

		if output.status.success() && reencoded_path.is_file() {
			match (cfg!(debug_assertions), tokio::fs::remove_file(path).await) {
				(_, Ok(())) => {}
				(true, Err(err)) => panic!("Failed to remove original video: {err}"),
				(false, Err(err)) => log::error!("Failed to remove original video: {err}"),
			}

			Ok(reencoded_path)
		} else {
			Err(std::io::Error::new(
				std::io::ErrorKind::Other,
				format!(
					"Exit status: {}\n\n=========== stderr ===========\n{}\n\n=========== stdout ===========\n{}",
					output.status,
					String::from_utf8_lossy(&output.stderr),
					String::from_utf8_lossy(&output.stdout)
				),
			))
		}
	}
}

struct YtDlpDaemonInner {
	client: reqwest::Client,
	yt_dlp: RwLock<YtDlp>,
	last_update_check: Mutex<Instant>,
}

#[derive(Clone)]
pub struct YtDlpDaemon(Arc<YtDlpDaemonInner>);
impl YtDlpDaemon {
	pub async fn new() -> Result<Self, anyhow::Error> {
		log::info!("Initializing yt-dlp daemon...");

		if Path::new("yt_dlp_out").exists() {
			tokio::fs::remove_dir_all("yt_dlp_out").await?;
		}

		Ok(Self(Arc::new(YtDlpDaemonInner {
			client: reqwest::Client::new(),
			yt_dlp: RwLock::new(YtDlp::new().await?),
			last_update_check: Mutex::new(Instant::now()),
		})))
	}

	pub async fn update(&self) -> Result<(), anyhow::Error> {
		log::info!("Automatic yt-dlp daemon update check...");

		let release = YtDlpRelease::latest().await?;

		let mut yt_dlp = self.0.yt_dlp.write().await;

		if release.tag_name == yt_dlp.tag_name {
			log::info!("yt-dlp daemon up-to-date!");
			return Ok(());
		} else {
			log::info!("yt-dlp daemon outdated, updating...");
		}

		*yt_dlp = YtDlp::download_release(release).await?;

		log::info!("yt-dlp daemon updated!");

		Ok(())
	}

	pub async fn download(&self, url: &str) -> Result<DownloadedMedia, anyhow::Error> {
		let path = uuid::Uuid::new_v4().to_string();
		let path = Path::new("yt_dlp_out").join(path).into_boxed_path();

		tokio::fs::create_dir_all("yt_dlp_out").await.context("creating yt_dlp_out directory")?;

		let url = self
			.0
			.client
			.get(url)
			.header("User-Agent", USER_AGENT)
			.send()
			.await?
			.error_for_status()?
			.url()
			.to_string();

		if let Some(photo_id) = tiktok::get_tiktok_photo_id_from_url(&url) {
			// TikTok slideshow

			let path = tiktok::extract_slideshow_images(photo_id, &path).await?;

			return Ok(DownloadedMedia {
				path: path.into_boxed_path(),
				url: None,
			});
		}

		self.update_check().await; // This will complete really quickly and do stuff in the background.

		self.0.yt_dlp.read().await.download(&url, &path.with_extension("mp4")).await
	}

	async fn update_check(&self) {
		let Ok(mut last_update_check) = self.0.last_update_check.try_lock() else {
			// Another thread is already checking for updates
			return;
		};

		if last_update_check.elapsed() > YT_DLP_UPDATE_CHECK_INTERVAL {
			*last_update_check = Instant::now();

			let this = self.clone();
			tokio::spawn(async move {
				if let Err(err) = this.update().await {
					log::error!("Failed to update yt-dlp: {}", err);
				}
			});
		}
	}
}

pub struct DownloadedMedia {
	pub path: Box<Path>,
	pub url: Option<Box<str>>,
}
impl Drop for DownloadedMedia {
	fn drop(&mut self) {
		log::info!("Deleting {}", self.path.display());

		if let Ok(rt) = tokio::runtime::Handle::try_current() {
			let path = self.path.clone();
			rt.spawn(async move { tokio::fs::remove_file(&path).await });
		} else {
			std::fs::remove_file(&self.path).ok();
		}
	}
}

#[derive(Debug, serde::Deserialize)]
struct YtDlpJsonDump {
	requested_downloads: Vec<YtDlpJsonDumpRequestedDownload>,
	url: String,
}

#[derive(Debug, serde::Deserialize)]
struct YtDlpJsonDumpRequestedDownload {
	url: String,
}
