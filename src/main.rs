mod commands;
mod flash_cards;

use commands::*;
use etcetera::app_strategy::{choose_app_strategy, AppStrategy, AppStrategyArgs};
use serenity::async_trait;
use serenity::client::{Client, Context, EventHandler};
use serenity::framework::standard::macros::{command, group};
use serenity::framework::standard::{Args, CommandResult, StandardFramework};
use serenity::model::channel::{Message, Reaction, ReactionType};
use serenity::model::gateway::Ready;
use std::convert::TryFrom;
use std::sync::Arc;
use tokio::fs;
use tokio::sync::RwLock;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let app_strategy = choose_app_strategy(AppStrategyArgs {
        top_level_domain: "io.github".to_string(),
        author: "arzg".to_string(),
        app_name: "g22-study-bot".to_string(),
    })?;

    let framework = StandardFramework::new()
        .configure(|c| c.prefix("~"))
        .group(&GENERAL_GROUP);

    let token = dotenv::var("DISCORD_TOKEN")?;

    let mut client = Client::builder(token)
        .event_handler(Handler::default())
        .framework(framework)
        .await?;

    {
        let calendar_path = app_strategy.in_data_dir("calendar");

        let calendar = if !calendar_path.exists() {
            fs::create_dir_all(app_strategy.data_dir()).await?;
            CalendarData::default()
        } else {
            bincode::deserialize(&fs::read(calendar_path).await?)?
        };

        let mut data = client.data.write().await;
        data.insert::<Calendar>(Arc::new(RwLock::new(calendar)));
        data.insert::<Config>(Arc::new(app_strategy.in_data_dir("calendar")));
    }

    client.start().await?;

    Ok(())
}

#[derive(Default)]
struct Handler {
    flash_card_submissions: RwLock<Vec<flash_cards::Submission>>,
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, _: Context, ready: Ready) {
        println!("{} is connected!", ready.user.name);
    }

    async fn message(&self, ctx: Context, message: Message) {
        if self.calendar_is_outdated(&ctx).await {
            self.remove_outdated_assignments(&ctx).await;
            if let Err(e) = self.refresh_calendar(&ctx).await {
                eprintln!("Error: {}", e);
            }
        }

        if let Err(e) = self.handle_message(&ctx, message).await {
            eprintln!("Error: {}", e);
        }
    }

    async fn reaction_add(&self, ctx: Context, reaction: Reaction) {
        match self.handle_flash_card_vote(&ctx, &reaction).await {
            Ok(true) => {}
            Ok(false) => {
                if let Err(e) = self.handle_assignment_vote(&ctx, reaction).await {
                    eprintln!("Error: {}", e);
                }
            }
            Err(e) => eprintln!("Error: {}", e),
        }
    }
}

impl Handler {
    pub(crate) async fn is_submission_accepted(
        &self,
        ctx: &Context,
        submission: &Message,
    ) -> anyhow::Result<bool> {
        let needed_votes = self.needed_votes(ctx, &submission).await?;

        let num_votes = submission
            .reaction_users(
                &ctx.http,
                ReactionType::Unicode("👍".to_string()),
                None,
                None,
            )
            .await?
            .len()
            - 1; // Because the bot has also reacted.

        let has_enough_votes_to_be_accepted = num_votes >= needed_votes;

        Ok(has_enough_votes_to_be_accepted)
    }

    async fn needed_votes(&self, ctx: &Context, submission: &Message) -> anyhow::Result<usize> {
        let for_review_channel = submission
            .channel_id
            .to_channel(&ctx.http)
            .await?
            .guild()
            .unwrap();

        let for_review_channel_members = for_review_channel.members(&ctx.cache).await?;

        // Account for the bot being a member of the channel.
        let num_voters = for_review_channel_members.len() - 1;

        let needed_votes = f32::from(u16::try_from(num_voters).unwrap()) / 2.0;
        let needed_votes = needed_votes.round() as u16;

        Ok(needed_votes.into())
    }
}

#[group]
#[commands(prune, calendar_insert)]
struct General;

#[command]
async fn prune(ctx: &Context, message: &Message, mut args: Args) -> CommandResult {
    let permissions = message
        .member(&ctx.http)
        .await?
        .permissions(&ctx.cache)
        .await?;

    if !permissions.manage_messages() {
        message
            .reply(&ctx.http, "You need to have Manage Messages to use prune.")
            .await?;

        return Ok(());
    }

    let num_messages_to_prune = args.single()?;

    let messages = message
        .channel_id
        .messages(&ctx.http, |get_messages| {
            get_messages.before(message).limit(num_messages_to_prune)
        })
        .await?;

    message
        .channel_id
        .delete_messages(&ctx.http, messages)
        .await?;

    message.delete(&ctx.http).await?;

    Ok(())
}
