use crate::Handler;
use serenity::client::Context;
use serenity::http::AttachmentType;
use serenity::model::channel::{Message, Reaction, ReactionType};
use serenity::model::id::{ChannelId, MessageId};
use serenity::model::misc::Mentionable;
use serenity::model::user::User;

const FOR_REVIEW_CHANNEL_ID: ChannelId = ChannelId(793720525250756639);
const FLASH_CARDS_CHANNEL_ID: ChannelId = ChannelId(771593804057673767);

impl Handler {
    pub(crate) async fn handle_message(
        &self,
        ctx: &Context,
        message: Message,
    ) -> anyhow::Result<()> {
        if message.author.bot {
            return Ok(());
        }

        let in_dm = message.guild_id.is_none();

        if !in_dm {
            return Ok(());
        }

        let deck = {
            let attachment = if let Some(attachment) = message.attachments.into_iter().next() {
                attachment
            } else {
                return Ok(());
            };

            if !attachment.filename.ends_with(".apkg") {
                // message
                //     .channel_id
                //     .send_message(&ctx.http, |create_message| {
                //         create_message.content("You need to send an .apkg")
                //     })
                //     .await?;

                return Ok(());
            }

            Deck {
                data: attachment.download().await?,
                filename: attachment.filename,
            }
        };

        let submission_msg = FOR_REVIEW_CHANNEL_ID
            .send_message(&ctx.http, |create_message| create_message.add_file(&deck))
            .await?;

        submission_msg
            .react(&ctx.http, ReactionType::Unicode("👍".to_string()))
            .await?;

        self.flash_card_submissions.write().await.push(Submission {
            deck,
            message_id: submission_msg.id,
            author: message.author,
            accepted: false,
        });

        Ok(())
    }

    pub(crate) async fn handle_flash_card_vote(
        &self,
        ctx: &Context,
        reaction: &Reaction,
    ) -> anyhow::Result<bool> {
        let submission_being_reacted_to = {
            let submissions = self.flash_card_submissions.read().await;

            submissions
                .iter()
                .find(|submission| submission.message_id == reaction.message_id)
                .cloned()
        };

        if let Some(submission_being_reacted_to) = submission_being_reacted_to {
            if submission_being_reacted_to.accepted {
                println!("Submission was already accepted");
                return Ok(true);
            }

            let submission_msg = reaction.message(&ctx.http).await?;

            if self.is_submission_accepted(&ctx, &submission_msg).await? {
                println!("Submission accepted!");
                self.send_submission_to_flash_cards(&ctx).await?;
                self.add_contributor_role(&ctx).await?;
                self.mark_last_submission_as_accepted().await;
            }

            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn send_submission_to_flash_cards(&self, ctx: &Context) -> anyhow::Result<()> {
        let submissions = self.flash_card_submissions.read().await;
        let last_submission = submissions.last().unwrap();

        FLASH_CARDS_CHANNEL_ID
            .send_files(
                &ctx.http,
                std::iter::once(&last_submission.deck),
                |create_message| {
                    create_message
                        .content(format!("Submitted by {}", last_submission.author.mention()))
                },
            )
            .await?;

        Ok(())
    }

    async fn add_contributor_role(&self, ctx: &Context) -> anyhow::Result<()> {
        let submissions = self.flash_card_submissions.read().await;
        let last_submission = submissions.last().unwrap();

        let guild_id = ctx.cache.guilds().await[0];

        let guild_roles = ctx.cache.guild_roles(guild_id).await.unwrap();

        let contributor_role_id = guild_roles
            .iter()
            .find_map(|(role_id, role)| {
                if role.name == "contributor" {
                    Some(role_id)
                } else {
                    None
                }
            })
            .ok_or_else(|| anyhow::anyhow!("could not find contributor role"))?;

        ctx.http
            .add_member_role(
                guild_id.0,
                last_submission.author.id.0,
                contributor_role_id.0,
            )
            .await?;

        Ok(())
    }

    async fn mark_last_submission_as_accepted(&self) {
        self.flash_card_submissions
            .write()
            .await
            .last_mut()
            .unwrap()
            .accepted = true;
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Submission {
    deck: Deck,
    message_id: MessageId,
    author: User,
    accepted: bool,
}

#[derive(Debug, Clone)]
struct Deck {
    data: Vec<u8>,
    filename: String,
}

impl<'a> From<&'a Deck> for AttachmentType<'a> {
    fn from(deck: &'a Deck) -> Self {
        (deck.data.as_slice(), deck.filename.as_str()).into()
    }
}
