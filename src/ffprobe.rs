#![allow(clippy::get_first)]

use anyhow::Context;
use std::{path::Path, time::Duration};

use crate::discord::DISCORD_FILE_SIZE_LIMIT;

#[derive(Debug, PartialEq, Eq)]
pub enum MediaProbe {
	Corrupt,
	Probed { is_discord_compatible: bool, duration: Duration },
}
impl MediaProbe {
	pub async fn get(path: &Path) -> Result<Self, anyhow::Error> {
		let metadata = tokio::fs::metadata(path).await?;

		let output = tokio::process::Command::new(if cfg!(windows) { "ffprobe.exe" } else { "ffprobe" })
			.args([
				"-v",
				"error",
				"-show_entries",
				"stream=codec_type,codec_name",
				"-show_entries",
				"format=duration",
				"-of",
				"json",
			])
			.arg(path)
			.output()
			.await?;

		let stdout = String::from_utf8_lossy(&output.stdout);
		let stdout = stdout.as_ref();

		let stderr = String::from_utf8_lossy(&output.stderr);
		let stderr = stderr.as_ref();

		if stderr.contains("Packet corrupt") || stdout.contains("Packet corrupt") {
			return Ok(Self::Corrupt);
		}

		if !output.status.success() {
			return Err(anyhow::anyhow!(
				"Exit status: {}\n\n=========== stderr ===========\n{}\n\n=========== stdout ===========\n{}",
				output.status,
				stdout,
				stderr
			));
		}

		let output: FFProbeOutput = serde_json::from_str(stdout).context("Failed to parse ffprobe output")?;

		let is_discord_compatible = metadata.len() < DISCORD_FILE_SIZE_LIMIT
			// at least one video stream
			&& output.streams.iter().any(|stream| stream.codec_type == "video")
			// all video streams are h264 and all audio streams are aac
			&& output.streams.iter().all(|stream| {
				(stream.codec_type == "video" && stream.codec_name == "h264") ||
				(stream.codec_type == "audio" && stream.codec_name == "aac")
			});

		if cfg!(debug_assertions) && !is_discord_compatible {
			log::info!("Not compatible with Discord: filesize={} {:#?}", metadata.len(), output);
		}

		Ok(Self::Probed {
			is_discord_compatible,
			duration: Duration::from_secs_f64(output.format.duration.parse::<f64>().context("Failed to parse duration")?),
		})
	}
}

#[derive(serde::Deserialize, Debug)]
struct FFProbeOutput {
	streams: Vec<FFProbeStream>,
	format: FFProbeFormat,
}

#[derive(serde::Deserialize, Debug)]
struct FFProbeStream {
	codec_name: String,
	codec_type: String,
}

#[derive(serde::Deserialize, Debug)]
struct FFProbeFormat {
	duration: String,
}
