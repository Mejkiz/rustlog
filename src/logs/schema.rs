use anyhow::{anyhow, Context};
use chrono::TimeZone;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_repr::Serialize_repr;
use std::borrow::Cow;
use std::{collections::HashMap, fmt::Display};
use strum::EnumString;
use twitch::{Command, Tag};

const TIMESTAMP_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

#[derive(Serialize, Deserialize, Debug, JsonSchema)]
pub struct ChannelLogDate {
    pub year: u32,
    pub month: u32,
    pub day: u32,
}

impl Display for ChannelLogDate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{:0>2}-{:0>2}", self.year, self.month, self.day)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct UserLogDate {
    pub year: u32,
    pub month: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserIdentifier<'a> {
    User(&'a str),
    UserId(&'a str),
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelIdentifier<'a> {
    Channel(&'a str),
    ChannelId(&'a str),
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Message<'a> {
    pub text: Cow<'a, str>,
    pub username: &'a str,
    pub display_name: &'a str,
    pub channel: &'a str,
    #[schemars(with = "String")]
    pub timestamp: DateTime<Utc>,
    pub id: &'a str,
    pub raw: &'a str,
    #[schemars(with = "i8")]
    pub r#type: MessageType,
    pub tags: HashMap<&'a str, Cow<'a, str>>,
}

#[derive(Serialize_repr, EnumString, Debug)]
#[repr(i8)]
#[strum(serialize_all = "UPPERCASE")]
pub enum MessageType {
    // Whisper = 0,
    PrivMsg = 1,
    ClearChat = 2,
    // RoomState = 3,
    UserNotice = 4,
    // UserState = 5,
    // Notice = 6,
    ClearMsg = 13,
}

impl<'a> Message<'a> {
    pub fn from_irc_message(irc_message: &'a twitch::Message) -> anyhow::Result<Self> {
        let tags = irc_message.tags().context("Message has no tags")?;
        let channel = irc_message.channel().context("Missing channel")?;

        let raw_timestamp = tags
            .get(&Tag::TmiSentTs)
            .context("Missing timestamp tag")?
            .parse::<i64>()
            .context("Invalid timestamp")?;
        let timestamp = Utc
            .timestamp_millis_opt(raw_timestamp)
            .single()
            .context("Invalid timestamp")?;

        let response_tags = tags
            .into_iter()
            .map(|(key, value)| (key.as_str(), Cow::Borrowed(*value)))
            .collect();

        match irc_message.command() {
            Command::Privmsg => {
                let raw_text = irc_message.params().context("Privmsg has no params")?;
                let text = extract_message_text(&raw_text);

                let display_name = *tags
                    .get(&Tag::DisplayName)
                    .context("Missing display name tag")?;
                let username = irc_message
                    .prefix()
                    .context("Message has no prefix")?
                    .nick
                    .context("Missing nickname")?;
                let id = *tags.get(&Tag::Id).context("Missing message id tag")?;

                Ok(Self {
                    text: Cow::Borrowed(text),
                    username,
                    display_name,
                    channel,
                    timestamp,
                    id,
                    raw: irc_message.raw(),
                    r#type: MessageType::PrivMsg,
                    tags: response_tags,
                })
            }
            Command::Clearchat => {
                let mut username = None;

                let text = match irc_message.params() {
                    Some(user_login) => {
                        username = Some(user_login);

                        match tags.get(&Tag::BanDuration) {
                            Some(ban_duration) => {
                                format!(
                                    "{user_login} has been timed out for {ban_duration} seconds"
                                )
                            }
                            None => {
                                format!("{user_login} has been banned")
                            }
                        }
                    }
                    None => "Chat has been cleared".to_owned(),
                };

                Ok(Message {
                    text: Cow::Owned(text),
                    display_name: username.unwrap_or_default(),
                    username: username.unwrap_or_default(),
                    channel,
                    timestamp,
                    id: "",
                    raw: irc_message.raw(),
                    r#type: MessageType::ClearChat,
                    tags: response_tags,
                })
            }
            Command::UserNotice => {
                let system_message = tags
                    .get(&Tag::SystemMsg)
                    .context("System message tag missing")?;
                let system_message = twitch::unescape(system_message);

                let text = if let Some(user_message) = irc_message.params() {
                    let user_message = extract_message_text(&user_message);
                    Cow::Owned(format!("{system_message} {user_message}"))
                } else {
                    Cow::Owned(system_message)
                };

                let display_name = *tags
                    .get(&Tag::DisplayName)
                    .context("Missing display name tag")?;
                let username = *tags.get(&Tag::Login).context("Missing login tag")?;
                let id = *tags.get(&Tag::Id).context("Missing message id tag")?;

                let response_tags = response_tags
                    .into_iter()
                    .map(|(key, value)| (key, Cow::Owned(twitch::unescape(&value))))
                    .collect();

                Ok(Message {
                    text,
                    username,
                    display_name,
                    channel,
                    timestamp,
                    id,
                    raw: irc_message.raw(),
                    r#type: MessageType::UserNotice,
                    tags: response_tags,
                })
            }
            other => Err(anyhow!("Unsupported message type: {other:?}")),
        }
    }
}

impl<'a> Display for Message<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let timestamp = self.timestamp.format(TIMESTAMP_FORMAT);
        let channel = &self.channel;
        let username = &self.username;
        let text = &self.text;

        if !username.is_empty() {
            write!(f, "[{timestamp}] {channel} {username}: {text}")
        } else {
            write!(f, "[{timestamp}] {channel} {text}")
        }
    }
}

fn extract_message_text(message_text: &str) -> &str {
    let message_text = message_text.trim_start();
    let mut message_text = message_text.strip_prefix(':').unwrap_or(message_text);

    let is_action =
        message_text.starts_with("\u{0001}ACTION ") && message_text.ends_with('\u{0001}');
    if is_action {
        // remove the prefix and suffix
        message_text = &message_text[8..message_text.len() - 1]
    }

    message_text
}
