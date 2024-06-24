use crate::AppContext;
use serenity::{
	all::{
		Command, CommandInteraction, CreateAttachment, CreateCommand, CreateCommandOption, CreateInteractionResponse,
		CreateInteractionResponseMessage, ResolvedOption, ResolvedValue,
	},
	prelude::*,
};

pub async fn register(ctx: &Context) -> Result<(), anyhow::Error> {
	Command::create_global_command(
		ctx,
		CreateCommand::new("download")
			.description("Download a video from a website using yt-dlp and embed it in the channel")
			.add_option(CreateCommandOption::new(
				serenity::all::CommandOptionType::String,
				"url",
				"URL of the video",
			)),
	)
	.await?;

	Ok(())
}

pub async fn run(app_ctx: &AppContext, ctx: &Context, command: &CommandInteraction, options: &[ResolvedOption<'_>]) -> Result<(), anyhow::Error> {
	let Some(download_url) = options.first().and_then(|option| match (option.name, &option.value) {
		("url", ResolvedValue::String(url)) => Some(*url),
		_ => None,
	}) else {
		return command
			.create_response(
				ctx,
				CreateInteractionResponse::Message(CreateInteractionResponseMessage::new().ephemeral(true).content("URL is required")),
			)
			.await
			.map_err(Into::into);
	};

	let msg: Result<_, anyhow::Error> = async {
		let media = app_ctx.yt_dlp.download(download_url).await.map_err(|err| {
			log::error!("Failed to download {download_url} ({err})");
			err
		})?;

		Ok(CreateInteractionResponse::Message(
			CreateInteractionResponseMessage::new().add_file(CreateAttachment::path(&media.path).await?),
		))
	}
	.await;

	command
		.create_response(
			ctx,
			match msg {
				Ok(msg) => msg,
				Err(err) => {
					log::error!("Failed to download {download_url} ({err})");
					CreateInteractionResponse::Message(
						CreateInteractionResponseMessage::new()
							.ephemeral(true)
							.content("Failed to download a video from this URL!"),
					)
				}
			},
		)
		.await
		.map_err(Into::into)
}
