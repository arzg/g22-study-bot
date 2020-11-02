use crate::Handler;
use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use serenity::builder::CreateMessage;
use serenity::client::Context;
use serenity::framework::standard::macros::command;
use serenity::framework::standard::{Args, CommandResult};
use serenity::http::AttachmentType;
use serenity::model::channel::{Message, Reaction, ReactionType};
use serenity::model::id::ChannelId;
use serenity::model::id::MessageId;
use serenity::prelude::TypeMapKey;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use tokio::sync::RwLock;

const CALENDAR_CHANNEL_ID: ChannelId = ChannelId(771595328029589544);

#[command]
pub(crate) async fn calendar_insert(
    ctx: &Context,
    message: &Message,
    mut args: Args,
) -> CommandResult {
    let subject = args.single()?;
    let day = args.single()?;
    let month = args.single()?;

    let notifications = {
        let mut notifications = Vec::with_capacity(message.attachments.len());

        for attachment in &message.attachments {
            notifications.push(Notification {
                data: attachment.download().await?,
                filename: attachment.filename.clone(),
            });
        }

        notifications
    };

    let assignment = Assignment {
        subject,
        due_date: NaiveDate::from_ymd(2020, month, day),
        notifications,
        accepted: false,
    };

    let guild = CALENDAR_CHANNEL_ID
        .to_channel(&ctx.http)
        .await?
        .guild()
        .unwrap()
        .guild(&ctx.cache)
        .await
        .unwrap();

    let for_review_channel = guild
        .channel_id_from_name(&ctx.cache, "for-review")
        .await
        .unwrap();

    let for_review_message = for_review_channel
        .send_message(&ctx.http, |create_message| {
            assignment.create_message(create_message)
        })
        .await?;

    {
        let calendar = {
            let data = ctx.data.read().await;
            Arc::clone(data.get::<Calendar>().unwrap())
        };

        let mut calendar = calendar.write().await;

        calendar
            .assignments
            .insert(for_review_message.id, assignment);
    }

    for_review_message
        .react(&ctx.http, ReactionType::Unicode("👍".to_string()))
        .await?;

    Ok(())
}

impl Handler {
    pub(crate) async fn handle_assignment_vote(
        &self,
        ctx: &Context,
        reaction: Reaction,
    ) -> anyhow::Result<()> {
        if reaction.emoji != ReactionType::Unicode("👍".to_string()) {
            return Ok(());
        }

        let calendar = {
            let data = ctx.data.read().await;
            Arc::clone(data.get::<Calendar>().unwrap())
        };
        let mut calendar = calendar.write().await;

        let submission_being_reacted_to = match calendar.assignments.get_mut(&reaction.message_id) {
            Some(submission) => submission,
            None => return Ok(()),
        };

        if submission_being_reacted_to.accepted {
            return Ok(());
        }

        let reaction_message = reaction.message(&ctx.http).await?;

        if self.is_submission_accepted(ctx, &reaction_message).await? {
            submission_being_reacted_to.accepted = true;

            CALENDAR_CHANNEL_ID
                .send_message(&ctx.http, |create_message| {
                    create_message
                        .content(reaction_message.content)
                        .add_files(submission_being_reacted_to.notifications.clone())
                })
                .await?;
        }

        Ok(())
    }
}

pub(crate) struct Calendar;

impl TypeMapKey for Calendar {
    type Value = Arc<RwLock<CalendarData>>;
}

#[derive(Default, Serialize, Deserialize)]
pub(crate) struct CalendarData {
    assignments: HashMap<MessageId, Assignment>,
}

impl fmt::Display for CalendarData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let num_assignments = self.assignments.len();
        let is_at_end = |idx| idx == num_assignments - 1;

        for (idx, assignment) in self
            .assignments
            .values()
            .filter(|assignment| assignment.accepted)
            .enumerate()
        {
            write!(f, "{}", assignment)?;

            if !is_at_end(idx) {
                writeln!(f)?;
            }
        }

        Ok(())
    }
}

#[derive(Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub(crate) struct Assignment {
    subject: String,
    due_date: NaiveDate,
    notifications: Vec<Notification>,
    accepted: bool,
}

impl fmt::Display for Assignment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: Due on {}/{}.",
            self.subject,
            self.due_date.day(),
            self.due_date.month(),
        )
    }
}

impl Assignment {
    fn create_message<'a, 'b>(
        &'b self,
        create_message: &'b mut CreateMessage<'a>,
    ) -> &'b mut CreateMessage<'a> {
        create_message
            .content(self)
            .add_files(self.notifications.clone().into_iter())
    }
}

#[derive(Clone, Ord, PartialOrd, Eq, PartialEq, Hash, Serialize, Deserialize)]
struct Notification {
    data: Vec<u8>,
    filename: String,
}

impl From<Notification> for AttachmentType<'_> {
    fn from(notification: Notification) -> Self {
        Self::Bytes {
            data: notification.data.into(),
            filename: notification.filename,
        }
    }
}
