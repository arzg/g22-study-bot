use super::Config;
use crate::Handler;
use chrono::{Datelike, Local, NaiveDate};
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
use tokio::fs;
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

        {
            let calendar = {
                let data = ctx.data.read().await;
                Arc::clone(data.get::<Calendar>().unwrap())
            };
            let calendar = calendar.read().await;
            let submission_being_reacted_to =
                calendar.assignments.get(&reaction.message_id).unwrap();

            if submission_being_reacted_to.accepted {
                return Ok(());
            }
        }

        let reaction_message = reaction.message(&ctx.http).await?;

        if self.is_submission_accepted(ctx, &reaction_message).await? {
            let calendar = {
                let data = ctx.data.read().await;
                Arc::clone(data.get::<Calendar>().unwrap())
            };

            {
                let mut calendar = calendar.write().await;

                calendar
                    .assignments
                    .get_mut(&reaction_message.id)
                    .unwrap()
                    .accepted = true;
            }

            self.refresh_calendar(ctx).await?;
        }

        Ok(())
    }

    pub(crate) async fn refresh_calendar(&self, ctx: &Context) -> anyhow::Result<()> {
        let calendar = {
            let data = ctx.data.read().await;
            Arc::clone(data.get::<Calendar>().unwrap())
        };

        let messages = CALENDAR_CHANNEL_ID
            .messages(&ctx.http, |get_messages| get_messages)
            .await?;

        for message in messages {
            CALENDAR_CHANNEL_ID
                .delete_message(&ctx.http, message)
                .await?;
        }

        let calendar = calendar.read().await;

        for assignment in calendar.assignments.values() {
            CALENDAR_CHANNEL_ID
                .send_message(&ctx.http, |create_message| {
                    assignment.create_message(create_message)
                })
                .await?;
        }

        let config_dir = {
            let data = ctx.data.read().await;
            Arc::clone(data.get::<Config>().unwrap())
        };

        fs::write(config_dir.as_path(), bincode::serialize(&*calendar)?).await?;

        Ok(())
    }

    pub(crate) async fn calendar_is_outdated(&self, ctx: &Context) -> bool {
        let calendar = {
            let data = ctx.data.read().await;
            Arc::clone(data.get::<Calendar>().unwrap())
        };

        let calendar = calendar.read().await;

        let now = Local::today().naive_local();

        calendar
            .assignments
            .values()
            .any(|assignment| assignment.due_date < now)
    }

    pub(crate) async fn remove_outdated_assignments(&self, ctx: &Context) {
        let calendar = {
            let data = ctx.data.write().await;
            Arc::clone(data.get::<Calendar>().unwrap())
        };

        let mut calendar = calendar.write().await;

        let now = Local::today().naive_local();
        calendar
            .assignments
            .retain(|_, assignment| assignment.due_date >= now);
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
