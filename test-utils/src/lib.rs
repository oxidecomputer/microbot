use microbot::{MatrixMessenger, MatrixMessengerSignals};
use matrix_sdk::{
    self, config::SyncSettings, ruma::{
        api::client::{
            account::register::v3::Request as RegistrationRequest,
            room::create_room::v3::Request as CreateRoomRequest,
        },
        assign,
        events::room::message::RoomMessageEventContent,
        OwnedRoomId, OwnedUserId,
    }, Client, RoomState
};
use ruma::api::client::{error::ErrorKind, uiaa};
use tokio::{sync::watch::Receiver, task::JoinHandle};

pub struct TestSetup {
    pub client: Client,
    pub room: OwnedRoomId,
    pub bots: Vec<OwnedUserId>,
}

pub async fn make_client(homeserver: &str) -> Client {
    let url = matrix_sdk::reqwest::Url::parse(homeserver)
        .expect("Failed to parse homeserver url");
    let client = Client::new(url).await.expect("Failed to construct client");

    client
}

pub async fn register(
    homeserver: &str,
    user: &str,
    password: &str,
) -> Client {
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

pub async fn setup(
    homeserver: &str,
    sender: &str,
    bots: &[&str],
) -> TestSetup {
    let mut bot_ids = vec![];

    for bot in bots {
        let client = register(homeserver, bot, bot).await;
        client.matrix_auth().login_username(&bot, &bot).await.expect("Bot failed to log in");
        bot_ids.push(client.user_id().expect("Failed to get bot id from client").to_owned());
    }

    let sender_client = register(homeserver, sender, sender).await;

    sender_client
        .matrix_auth()
        .login_username(sender, sender)
        .await
        .expect("Failed to log in");

    let request = assign!(CreateRoomRequest::new(), { invite: bot_ids.clone() });

    let room_resp = sender_client
        .create_room(request)
        .await
        .expect("Failed to create test room");

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

pub async fn spawn_bot(mut bot: MatrixMessenger) -> (JoinHandle<()>, Receiver<MatrixMessengerSignals>) {
    let mut signals = bot.signals();

    let bot_handle = tokio::spawn(async move {
        bot.start().await.expect("Bot failed to run to completion");
    });

    // Wait for the bot to register its command handlers
    while signals.changed().await.is_ok() {
        if *signals.borrow() == MatrixMessengerSignals::RegisterHandlers {
            break;
        }
    }

    (bot_handle, signals)
}

impl TestSetup {
    pub async fn send_cmd(
        &self,
        cmd: &str,
        message: &str,
    ) {
        let room = self
            .client
            .get_room(&self.room)
            .expect("Failed to find requested room");

        tracing::debug!(?cmd, ?message, "Sending test command");

        if room.state() == RoomState::Joined {
            room.send(
                RoomMessageEventContent::text_plain(format!(
                    "!{} {}",
                    cmd, message
                )),
            )
            .await
            .expect("Failed to send test command");
        } else {
            panic!("Attempted to send command to a room that sender has not joined")
        }
    }
}