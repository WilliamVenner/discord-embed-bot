use std::path::Path;
use tokio::{io::AsyncWriteExt, process::Command};

// TODO for slideshows with one image, just output the image

pub async fn extract_slideshow_images(photo_id: &str, out: impl AsRef<Path>) -> Result<(), anyhow::Error> {
	let api_url = format!("https://www.tiktok.com/api/item/detail/?aid=1988&app_language=en&app_name=tiktok_web&browser_language=en-GB&browser_name=Mozilla&browser_online=true&browser_platform=Win32&browser_version=5.0%20(Windows%20NT%2010.0%3B%20Win64%3B%20x64)%20AppleWebKit%2F537.36%20(KHTML,%20like%20Gecko)%20Chrome%2F132.0.0.0%20Safari%2F537.36&channel=tiktok_web&cookie_enabled=false&coverFormat=2&data_collection_enabled=false&device_id=7461615928682841622&device_platform=web_pc&focus_state=true&from_page=user&history_len=2&is_fullscreen=false&is_page_visible=true&language=en&odinId=7461615911201063958&os=windows&priority_region=&referer=&region=GB&screen_height=1314&screen_width=2562&tz_name=Europe%2FLondon&user_is_login=false&webcast_language=en&itemId={}", photo_id);

	let xbogus = {
		let mut node = Command::new("node")
			.arg("-")
			.stdin(std::process::Stdio::piped())
			.stdout(std::process::Stdio::piped())
			.stderr(std::process::Stdio::piped())
			.spawn()?;

		node.stdin
			.take()
			.unwrap()
			.write_all(
				format!(r#"console.log(require('xbogus')({:?}, 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/132.0.0.0 Safari/537.36'));"#,
					api_url
				)
				.as_bytes(),
			)
			.await?;

		let output = node.wait_with_output().await?;
		if !output.status.success() {
			return Err(anyhow::anyhow!(
				"Exit status: {}\n\n=========== stderr ===========\n{}\n\n=========== stdout ===========\n{}",
				output.status,
				String::from_utf8_lossy(&output.stderr),
				String::from_utf8_lossy(&output.stdout)
			));
		}

		String::from_utf8_lossy(&output.stdout).trim().to_string()
	};

	let api_data = tiktok_http_get(&format!("{api_url}&X-Bogus={xbogus}"))
		.send()
		.await?
		.json::<serde_json::Value>()
		.await?;

	let images = (|| api_data.get("itemInfo")?.get("itemStruct")?.get("imagePost")?.get("images")?.as_array())()
		.ok_or_else(|| anyhow::anyhow!("Failed to extract images"))?
		.iter()
		.filter_map(|image| image.get("imageURL")?.get("urlList")?.as_array()?.first()?.as_str())
		.collect::<Vec<_>>();

	let music = (|| api_data.get("itemInfo")?.get("itemStruct")?.get("music")?.get("playUrl")?.as_str())();

	if images.is_empty() {
		return Err(anyhow::anyhow!("No images found"));
	}

	generate_slideshow_video(out, images, music).await?;

	Ok(())
}

fn tiktok_http_get(url: &str) -> reqwest::RequestBuilder {
	static TIKTOK_HTTP: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(reqwest::Client::new);

	TIKTOK_HTTP
		.get(url)
		.header(
			"User-Agent",
			"Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/132.0.0.0 Safari/537.36",
		)
		.header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
		.header("Accept-Language", "en-US,en;q=0.5")
		.header("Sec-Fetch-Mode", "navigate")
		.header("Accept-Encoding", "gzip, deflate, br")
}

pub fn get_tiktok_photo_id_from_url(url: &str) -> Option<&str> {
	Some(
		regex::RegexBuilder::new(r#"https?://www\.tiktok\.com/@[\w.-]+/photo/(\d+)"#)
			.build()
			.unwrap()
			.captures(url)?
			.get(1)
			.unwrap()
			.as_str(),
	)
}

async fn generate_slideshow_video(out: impl AsRef<Path>, images: Vec<&str>, music: Option<&str>) -> Result<(), anyhow::Error> {
	let mut ffmpeg = Command::new("ffmpeg");

	ffmpeg
		.stdin(std::process::Stdio::piped())
		.stdout(std::process::Stdio::piped())
		.stderr(std::process::Stdio::piped())
		.args([
			"-f",
			"concat",
			"-safe",
			"0",
			"-protocol_whitelist",
			"file,http,tcp,https,tls,fd",
			"-i",
			"-",
		]);

	if let Some(music) = music {
		ffmpeg.args(["-i", music]);
	}

	ffmpeg.args([
		"-map",
		"0:v",
		"-map",
		"1:a",
		//"-vf",
		//"scale=1280:720:force_original_aspect_ratio=decrease:eval=frame,pad=1280:720:-1:-1:eval=frame,format=yuv420p",
		"-filter_complex",
		"[1:0] apad",
		"-shortest",
	]);

	ffmpeg.arg(out.as_ref());

	let mut ffmpeg = ffmpeg.spawn()?;

	let images = format!(
		"{}file '{}'\nduration 0",
		images.iter().map(|url| format!("file '{url}'\nduration 2\n")).collect::<String>(),
		images.last().unwrap() // Add an extra image to prevent the last image from being cut off
	);

	ffmpeg.stdin.take().unwrap().write_all(images.as_bytes()).await?;

	let ffmpeg = ffmpeg.wait_with_output().await?;

	if !ffmpeg.status.success() {
		return Err(anyhow::anyhow!(
			"Exit status: {}\n\n=========== stderr ===========\n{}\n\n=========== stdout ===========\n{}",
			ffmpeg.status,
			String::from_utf8_lossy(&ffmpeg.stderr),
			String::from_utf8_lossy(&ffmpeg.stdout)
		));
	}

	Ok(())
}
