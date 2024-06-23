pub trait AsyncFileLockShared: Sized {
	async fn try_lock_shared(self) -> std::io::Result<tokio::fs::File>;
}
impl AsyncFileLockShared for tokio::fs::File {
	async fn try_lock_shared(self) -> std::io::Result<tokio::fs::File> {
		use fs2::FileExt;

		let exe = self.into_std().await;
		exe.try_lock_shared()?;
		Ok(tokio::fs::File::from_std(exe))
	}
}
