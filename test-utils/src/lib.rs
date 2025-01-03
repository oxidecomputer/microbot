// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use matrix_sdk::{
    self,
    config::SyncSettings,
    ruma::{
        api::client::{
            account::register::v3::Request as RegistrationRequest,
            room::create_room::v3::Request as CreateRoomRequest,
        },
        assign,
        events::room::message::RoomMessageEventContent,
        OwnedRoomId, OwnedUserId,
    },
    Client, RoomState,
};
use microbot::{MatrixMessenger, MatrixMessengerSignals};
use ruma::api::client::{error::ErrorKind, uiaa};
use tokio::{sync::watch::Receiver, task::JoinHandle};
use tracing::instrument;

pub struct TestSetup {
    pub client: Client,
    pub room: OwnedRoomId,
    pub bots: Vec<OwnedUserId>,
}

pub async fn make_client(homeserver: &str) -> Client {
    let url = matrix_sdk::reqwest::Url::parse(homeserver).expect("Failed to parse homeserver url");
    let client = Client::new(url).await.expect("Failed to construct client");

    client
}

#[instrument]
pub async fn register(homeserver: &str, user: &str, password: &str) -> Client {
    tracing::info!("Registering user");

    let client = make_client(homeserver).await;

    let request = assign!(RegistrationRequest::new(), {
        username: Some(user.to_string()),
        password: Some(password.to_string()),
        auth: Some(uiaa::AuthData::Dummy(
            uiaa::Dummy::new(),
        )),
    });

    client
        .matrix_auth()
        .register(request)
        .await
        .map(|_| ())
        .or_else(|err| {
            // Ignore errors reporting that the user has already been registered
            if err.client_api_error_kind() == Some(&ErrorKind::UserInUse) {
                return Ok(());
            }

            Err(err)
        })
        .expect("Failed to register user");

    client
}

#[instrument]
pub async fn setup(homeserver: &str, sender: &str, bots: &[&str]) -> TestSetup {
    let mut bot_ids = vec![];

    for bot in bots {
        let client = register(homeserver, bot, bot).await;
        bot_ids.push(
            client
                .user_id()
                .expect("Failed to get bot id from client")
                .to_owned(),
        );
    }

    tracing::info!("Registered bots");

    let sender_client = register(homeserver, sender, sender).await;
    let request = assign!(CreateRoomRequest::new(), { invite: bot_ids.clone() });

    let room_resp = sender_client
        .create_room(request)
        .await
        .expect("Failed to create test room");

    tracing::info!("Created room");

    sender_client
        .sync_once(SyncSettings::default())
        .await
        .expect("Failed to sync with server");

    TestSetup {
        client: sender_client,
        room: room_resp.room_id().to_owned(),
        bots: bot_ids,
    }
}

pub async fn spawn_bot(
    mut bot: MatrixMessenger,
) -> (JoinHandle<()>, Receiver<MatrixMessengerSignals>) {
    let user = bot.user().to_string();
    let signals = bot.signals();
    bot.start().await.expect("Bot failed to run to start");

    tracing::info!(user, signal = ?signals.borrow(), "Started bot");

    let handle: JoinHandle<()> = tokio::spawn(async move {
        bot.await.expect("Bot ran to completion");
    });

    tracing::info!(user, signal = ?signals.borrow(), "Bot spawned");

    (handle, signals)
}

impl TestSetup {
    pub async fn send_cmd(&self, cmd: &str, message: &str) {
        let room = self
            .client
            .get_room(&self.room)
            .expect("Failed to find requested room");

        tracing::debug!(?cmd, ?message, "Sending test command");

        if room.state() == RoomState::Joined {
            room.send(RoomMessageEventContent::text_plain(format!(
                "!{} {}",
                cmd, message
            )))
            .await
            .expect("Failed to send test command");
        } else {
            panic!("Attempted to send command to a room that sender has not joined")
        }
    }
}
