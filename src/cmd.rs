use crate::AppContext;
use serenity::{
	all::{
		Command, CommandInteraction, CreateAttachment, CreateCommand, CreateCommandOption, CreateInteractionResponse,
		CreateInteractionResponseFollowup, CreateInteractionResponseMessage, ResolvedOption, ResolvedValue,
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
			))
			.kind(serenity::all::CommandType::ChatInput)
			.add_integration_type(serenity::all::InstallationContext::User)
			.add_integration_type(serenity::all::InstallationContext::Guild)
			.contexts(vec![
				serenity::model::application::InteractionContext::Guild,
				serenity::model::application::InteractionContext::BotDm,
				serenity::model::application::InteractionContext::PrivateChannel,
			])
			.default_member_permissions(serenity::all::Permissions::SEND_MESSAGES),
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

	command
		.create_response(&ctx, CreateInteractionResponse::Defer(CreateInteractionResponseMessage::new()))
		.await?;

	let media = app_ctx.yt_dlp.download(download_url).await.map_err(|err| {
		log::error!("Failed to download {download_url} ({err})");
		err
	});

	command
		.create_followup(
			ctx,
			match &media {
				Ok(media) => CreateInteractionResponseFollowup::new().add_file(CreateAttachment::path(&media.path).await?),
				Err(err) => {
					log::error!("Failed to download {download_url} ({err})");

					CreateInteractionResponseFollowup::new()
						.ephemeral(true)
						.content("Failed to download a video from this URL!")
				}
			},
		)
		.await?;

	drop(media);

	Ok(())
}
