use serde::Deserialize;
use std::{borrow::Borrow, collections::BTreeSet, time::Duration};

#[derive(Debug)]
pub struct Releases(pub Vec<Release>);
impl Releases {
	pub async fn get(repo: &str, timeout: Duration) -> Result<Self, anyhow::Error> {
		Ok(Self(
			reqwest::Client::new()
				.get(format!("https://api.github.com/repos/{repo}/releases").as_str())
				.timeout(timeout)
				.header("User-Agent", "based-ffmpreg")
				.send()
				.await?
				.json()
				.await?,
		))
	}
}

#[derive(Deserialize, Debug)]
pub struct Release {
	pub tag_name: Box<str>,
	pub assets: BTreeSet<Asset>,
	pub prerelease: bool,
	pub draft: bool,
}

#[derive(Deserialize, Debug)]
pub struct Asset {
	pub name: Box<str>,
	pub browser_download_url: Box<str>,
	pub size: u64,
}
impl Borrow<str> for Asset {
	#[inline(always)]
	fn borrow(&self) -> &str {
		&self.name
	}
}
impl PartialEq for Asset {
	#[inline(always)]
	fn eq(&self, other: &Self) -> bool {
		self.name == other.name
	}
}
impl Eq for Asset {}
impl PartialOrd for Asset {
	#[inline(always)]
	fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
		Some(self.cmp(other))
	}
}
impl Ord for Asset {
	#[inline(always)]
	fn cmp(&self, other: &Self) -> std::cmp::Ordering {
		self.name.cmp(&other.name)
	}
}
