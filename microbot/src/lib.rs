// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use command::CommandHandlers;
use context::MessengerContext;
use http::Extensions;
use matrix_sdk::{
    config::SyncSettings, event_handler::Ctx, Client, ClientBuildError, LoopCtrl, Room, RoomState,
};
use message::{CommandMessageParser, IntoCommand};
use ruma::{
    events::{
        room::{member::RoomMemberEventContent, message::RoomMessageEventContent},
        MessageLikeEventContent, OriginalSyncMessageLikeEvent, StrippedStateEvent,
    },
    OwnedUserId,
};
use serde::Deserialize;
use std::{
    future::Future,
    sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard},
    time::{Duration, Instant},
};
use thiserror::Error;
use tokio::{
    sync::watch::{Receiver, Sender},
    task::JoinHandle,
};
use tracing::instrument;

mod command;
pub use command::{CommandArgs, CommandFn, CommandHandler};
mod context;
mod message;

const SYNC_CALL_TIMEOUT: u64 = 15;
const MESSAGE_AGE_LIMIT: u32 = 30_000;

#[derive(Debug, Error)]
pub enum MessengerError {
    #[error("Bot has already been started")]
    AlreadyStarted,
    #[error(transparent)]
    Builder(#[from] ClientBuildError),
    #[error(transparent)]
    Client(#[from] matrix_sdk::Error),
    #[error(transparent)]
    Config(#[from] matrix_sdk::IdParseError),
    #[error("Bot has not been started")]
    NotStarted,
    #[error("Invalid prefix")]
    PrefixConfig(regex::Error),
}

#[derive(Debug, Deserialize)]
pub struct MatrixConfig {
    pub url: String,
    pub user: String,
    pub password: String,
    pub display_name: String,
    pub command_prefix: Option<String>,
}

/// Signals that are sent out by the bot to allow for external monitoring of its behavior
#[derive(Debug, PartialEq)]
pub enum MatrixMessengerSignals {
    Create,
    RegisterHandlers,
    Start,
    Stop,
    Sync,
}

pub struct MatrixMessenger {
    config: MatrixConfig,
    context: MessengerContext,
    handle: Option<JoinHandle<Result<(), MessengerError>>>,
    handlers: Arc<RwLock<CommandHandlers>>,
    signal: Sender<MatrixMessengerSignals>,
    watch: Receiver<MatrixMessengerSignals>,
    last_synced: Arc<RwLock<Option<Instant>>>,
}

impl MatrixMessenger {
    pub fn new(config: MatrixConfig) -> Self {
        tracing::info!("Creating bot signals channel");
        let (tx, rx) = tokio::sync::watch::channel(MatrixMessengerSignals::Create);

        Self {
            config,
            context: MessengerContext::new(),
            handle: None,
            handlers: Arc::new(RwLock::new(CommandHandlers::new())),
            signal: tx,
            watch: rx,
            last_synced: Arc::new(RwLock::new(None)),
        }
    }

    #[instrument(skip(self), fields(user = self.config.user))]
    pub async fn start(&mut self) -> Result<(), MessengerError> {
        let client = Client::builder()
            .homeserver_url(&self.config.url)
            .build()
            .await?;

        tracing::info!("Logging in to server");
        client
            .matrix_auth()
            .login_username(&self.config.user, &self.config.password)
            .initial_device_display_name(&self.config.display_name)
            .await?;

        tracing::info!("Add room join handler");
        client.add_event_handler(Self::handle_autojoin_event);

        tracing::info!("Logged in. Starting initial room sync");
        let response = client.sync_once(SyncSettings::default()).await?;
        tracing::info!(?response, "Completed initial room sync");

        tracing::info!("Starting initial message sync");
        let response = client
            .sync_once(SyncSettings::default().token(response.next_batch))
            .await?;
        tracing::info!(?response, "Completed initial message sync");

        tracing::info!("Registering context data");
        let mut parser = CommandMessageParser::default();
        if let Some(prefix) = &self.config.command_prefix {
            parser
                .set_prefix(Some(prefix.clone()))
                .map_err(MessengerError::PrefixConfig)?;
        }
        let user_id = client.user_id().map(|id| id.to_owned());

        client.add_event_handler_context(user_id);
        client.add_event_handler_context(self.handlers.clone());
        client.add_event_handler_context(parser);
        client.add_event_handler_context(self.context.clone());

        tracing::info!("Registering event handler");
        client.add_event_handler(Self::handle_room_message_event::<RoomMessageEventContent>);

        self.signal.send(MatrixMessengerSignals::RegisterHandlers).expect("Failed to send signal. This should only ever happen if the internal receiver has gone missing");
        tracing::info!("Sent RegisterHandlers signal");

        let sync_client = client.clone();

        tracing::info!("Preparing sync task");
        let last_synced = Arc::new(RwLock::new(Instant::now()));
        let sync_monitor = last_synced.clone();

        let sync_signal = self.signal.clone();
        self.handle = Some(tokio::spawn(async move {
            tracing::info!("Spawning sync task");
            let stop_signal = sync_signal.clone();

            let settings = SyncSettings::default()
                .token(response.next_batch)
                .timeout(Duration::from_secs(SYNC_CALL_TIMEOUT));
            sync_client
                .sync_with_callback(settings, move |_| {
                    let sync_monitor = sync_monitor.clone();
                    let signal = sync_signal.clone();
                    async move {
                        let mut lock = sync_monitor.write().expect("Sync monitor lock failed");
                        tracing::trace!(time = ?*lock, "Handling sync");
                        *lock = Instant::now();
                        signal.send(MatrixMessengerSignals::Sync).expect("Failed to send signal. This should only ever happen if the internal receiver has gone missing");

                        LoopCtrl::Continue
                    }
                })
                .await?;

            stop_signal.send(MatrixMessengerSignals::Stop).expect("Failed to send signal. This should only ever happen if the internal receiver has gone missing");
            tracing::info!("Sending Stop signal");

            Ok::<(), MessengerError>(())
        }));

        Ok(())
    }

    pub fn handle(&self) -> Result<&JoinHandle<Result<(), MessengerError>>, MessengerError> {
        if let Some(handle) = &self.handle {
            Ok(handle)
        } else {
            Err(MessengerError::NotStarted)
        }
    }

    #[instrument(skip(self), fields(user = self.config.user))]
    pub fn abort(&mut self) -> Result<(), MessengerError> {
        if let Some(handle) = &self.handle {
            handle.abort();
            Ok(())
        } else {
            Err(MessengerError::NotStarted)
        }
    }

    /// Name of the user that the bot is configured to operate as
    pub fn user(&self) -> &str {
        &self.config.user
    }

    /// Returns a signal receiver that can be used to monitor the behaviors of the bot while it is
    /// executing
    pub fn signals(&self) -> Receiver<MatrixMessengerSignals> {
        self.watch.clone()
    }

    /// Returns the instant of when this bot successfully synced with the configured server
    pub fn last_synced(
        &self,
    ) -> Result<Option<Instant>, PoisonError<RwLockReadGuard<'_, Option<Instant>>>> {
        self.last_synced.read().map(|item| *item)
    }

    /// Handles accepting room invites
    async fn handle_autojoin_event(
        _event: StrippedStateEvent<RoomMemberEventContent>,
        room: Room,
        _client: Client,
    ) {
        if room.state() != RoomState::Joined {
            tracing::info!(?room, "Attempting to join room");

            match room.join().await {
                Ok(room) => tracing::info!(?room, "Joined room"),
                Err(err) => tracing::error!(?room, ?err, "Failed to join room"),
            }
        }
    }

    /// Performs the actual message parsing and handling. When a message is seen, the bot will
    /// attempt ot parse it into a [Command]. If it can, then it will look for the handlers that
    /// is registered for that command and run it. Only a single handler can be registered for a
    /// given command at a time
    async fn handle_room_message_event<T>(
        event: OriginalSyncMessageLikeEvent<T>,
        room: Room,
        client: Client,
        handlers: Ctx<Arc<RwLock<CommandHandlers>>>,
        extensions: Ctx<MessengerContext>,
        parser: Ctx<CommandMessageParser>,
        bot_user: Ctx<Option<OwnedUserId>>,
    ) where
        T: MessageLikeEventContent,
        OriginalSyncMessageLikeEvent<T>: IntoCommand + std::fmt::Debug,
        <OriginalSyncMessageLikeEvent<T> as IntoCommand>::Error: std::fmt::Debug,
    {
        tracing::debug!("Handle room event");

        if room.state() == RoomState::Joined {
            let expired = event
                .unsigned
                .age
                .map(|seconds| seconds.abs() > MESSAGE_AGE_LIMIT.into())
                .unwrap_or(true);

            // If this event has occured too far in the past (or the future) then we drop the event
            if !expired {
                let parsed = event.into_command(&parser);

                match parsed {
                    Ok(command) => {
                        tracing::info!(?command, "Parsed command");

                        if bot_user
                            .as_ref()
                            .map(|bot_user| *bot_user != command.sender)
                            .unwrap_or(false)
                        {
                            // We successfully parsed the incoming room event into its parts, a "command", and the remaining "message" text
                            let fut = match handlers.read() {
                                Ok(handlers) => match handlers.get(&command.command) {
                                    Some(handler) => Some(handler(CommandArgs {
                                        command,
                                        room,
                                        client,
                                        context: extensions.clone(),
                                    })),
                                    None => {
                                        tracing::info!(
                                            ?command,
                                            "Did not find handler for command"
                                        );
                                        None
                                    }
                                },
                                Err(err) => {
                                    tracing::error!(
                                        ?err,
                                        ?command,
                                        "Did not find handler for command"
                                    );
                                    None
                                }
                            };

                            if let Some(fut) = fut {
                                fut.await;
                            }
                        } else {
                            tracing::info!("Ignoring command that was sent by this bot")
                        }
                    }
                    Err(err) => {
                        tracing::debug!(?err, "Failed to parse event");
                    }
                }
            } else {
                tracing::warn!(?event.unsigned.age, "Event occured too far in the past or future to process");
            }
        }
    }

    /// Add a type `T` to the shared context that is passed to all command handler
    /// invocations. If data of type `T` is already stored in the shared context
    /// then the old data be replaced.
    pub fn insert_data<T>(
        &self,
        data: T,
    ) -> Result<Option<Arc<T>>, PoisonError<RwLockWriteGuard<'_, Extensions>>>
    where
        T: Send + Sync + 'static,
    {
        self.context.insert::<T>(data)
    }

    /// Lookup data of a given type `T` from the shared command handler context
    pub fn get_data<T>(
        &self,
    ) -> Result<Option<Arc<T>>, PoisonError<RwLockReadGuard<'_, Extensions>>>
    where
        T: Send + Sync + 'static,
    {
        self.context.get::<T>()
    }

    /// Register as a command handler. Handlers are uniquely defined by their associated
    /// command. Inserting a handler for an already registered command will replace the
    /// existing handler.
    ///
    /// Handlers need to implement [`CommandFn`] to be registered. This trait is generally
    /// derived as a blanket impl for async functions that accept a single [`CommandArgs`]
    /// argument.
    pub fn register_command<C>(
        &self,
        command: String,
        handler: C,
    ) -> Result<(), PoisonError<RwLockWriteGuard<'_, CommandHandlers>>>
    where
        C: CommandFn,
        <C::Response as Future>::Output: Sized,
    {
        self.handlers.write()?.insert(
            command.clone(),
            Box::new(move |args: CommandArgs| {
                let fut = handler.run(args);
                Box::pin(async move {
                    fut.await;
                })
            }),
        );

        tracing::info!(command, self.config.user, "Registered command");

        Ok(())
    }
}
