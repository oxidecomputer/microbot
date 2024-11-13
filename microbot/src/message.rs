// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use matrix_sdk::ruma::{
    events::room::message::{MessageType, OriginalSyncRoomMessageEvent, TextMessageEventContent},
    OwnedUserId,
};
use regex::Regex;

#[derive(Debug)]
pub struct MessageParseFailure {}

/// A command that has been created from a Matrix event. This is passed as part of a
/// [`CommandArgs`](crate::CommandArgs) to a [`CommandFn`](crate::CommandFn) when the association command string has been matched
#[derive(Debug, Clone)]
pub struct Command {
    /// The command that was matched (excluding the set prefix)
    pub command: String,

    /// The remaining text of the message after removing the prefix, the command, and any
    /// remaining leading whitespace
    pub message: String,

    /// The original sender of the message
    pub sender: OwnedUserId,
}

#[derive(Debug, Clone)]
pub(crate) struct CommandMessage {
    pub command: String,
    pub message: String,
}

pub(crate) trait IntoCommand {
    type Error;
    fn into_command(self, parser: &CommandMessageParser) -> Result<Command, Self::Error>;
}

impl IntoCommand for OriginalSyncRoomMessageEvent {
    type Error = MessageParseFailure;

    fn into_command(self, parser: &CommandMessageParser) -> Result<Command, Self::Error> {
        if let Some(message) = self.extract_message() {
            parser
                .parse(message)
                .map(|cmd_message| Command {
                    command: cmd_message.command,
                    message: cmd_message.message,
                    sender: self.sender,
                })
                .ok_or(MessageParseFailure {})
        } else {
            Err(MessageParseFailure {})
        }
    }
}

pub(crate) struct CommandMessageParser {
    prefix: Option<String>,
    command_pattern: Regex,
}

#[derive(Debug)]
pub(crate) struct CommandMessageParserError {
    pub inner: regex::Error,
}

impl From<regex::Error> for CommandMessageParserError {
    fn from(err: regex::Error) -> Self {
        Self { inner: err }
    }
}

impl std::fmt::Display for CommandMessageParserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Failed to construct command message parser: {}",
            self.inner
        )
    }
}

impl std::error::Error for CommandMessageParserError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.inner)
    }
}

impl Default for CommandMessageParser {
    fn default() -> Self {
        Self {
            command_pattern: Regex::new(r"(\w+)").expect("Known valid regex failed to compile"),
            prefix: None,
        }
    }
}

impl CommandMessageParser {
    #[allow(dead_code)]
    pub fn prefix(&self) -> Option<&str> {
        self.prefix.as_deref()
    }

    pub fn set_prefix(&mut self, prefix: Option<String>) -> Result<&mut Self, regex::Error> {
        self.command_pattern = Regex::new(&format!(r"^{}(\w+)", prefix.as_deref().unwrap_or("")))?;

        self.prefix = prefix;
        Ok(self)
    }

    pub(crate) fn parse(&self, message: &str) -> Option<CommandMessage> {
        let captures = self.command_pattern.captures(message);

        if let Some(captures) = captures {
            if let Some(command) = captures.get(1) {
                let command = command.as_str().to_string();
                let remaining_message = message
                    .trim_start_matches(self.prefix.as_deref().unwrap_or(""))
                    .trim_start_matches(&command)
                    .trim_start();

                return Some(CommandMessage {
                    command,
                    message: remaining_message.to_string(),
                });
            } else {
                tracing::debug!("Regex matched but not captures found");
            }
        } else {
            tracing::debug!("Failed to capture any commands");
        }

        None
    }
}

pub trait ExtractMessage {
    fn extract_message(&self) -> Option<&str>;
}

impl ExtractMessage for OriginalSyncRoomMessageEvent {
    fn extract_message(&self) -> Option<&str> {
        match &self.content.msgtype {
            MessageType::Text(TextMessageEventContent { body, .. }) => Some(body),
            other => {
                tracing::debug!(?other, "Skipping unsupported message");
                None
            }
        }
    }
}
