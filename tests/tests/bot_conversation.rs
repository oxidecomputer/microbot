// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use microbot::{CommandArgs, MatrixConfig, MatrixMessenger};
use microbot_test_utils::{setup, spawn_bot};
use rand::{distributions::Alphanumeric, Rng};
use ruma::events::room::message::RoomMessageEventContent;
use tokio::sync::{
    mpsc,
    mpsc::{Receiver, Sender},
};
use tracing_subscriber::filter::EnvFilter;

static HOMESERVER: &'static str = "http://localhost:8008";

// This test configures two bots and an initial sender that will send an initial command.
// In response to this command the two bots will command messages to the room triggering
// handlers of each other.

async fn configure_bot(bot_name: &str) -> (MatrixMessenger, Receiver<bool>) {
    let (tx, rx) = mpsc::channel::<bool>(1);

    let bot = MatrixMessenger::new(MatrixConfig {
        url: HOMESERVER.to_string(),
        user: bot_name.to_string(),
        password: bot_name.to_string(),
        display_name: bot_name.to_string(),
        command_prefix: Some("!".to_string()),
    });

    bot.insert_data(tx).expect("Failed to add bot context data");

    (bot, rx)
}

#[tokio::test]
async fn test_bot_conversation() {
    tracing_subscriber::fmt()
        .pretty()
        .with_env_filter(EnvFilter::default())
        .with_test_writer()
        .init();

    let sender = format!(
        "test-sender-{}",
        rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(24)
            .map(|b| char::from(b))
            .collect::<String>()
    );
    let bot1 = format!(
        "test-bot1-{}",
        rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(24)
            .map(|b| char::from(b))
            .collect::<String>()
    );
    let bot2 = format!(
        "test-bot2-{}",
        rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(24)
            .map(|b| char::from(b))
            .collect::<String>()
    );

    // This timeout ensures that the test fails if communication between the bots does not succeed or
    // if connections to the server hang
    tokio::time::timeout(tokio::time::Duration::from_secs(30), async {
        let setup = setup(HOMESERVER, &sender, &[&bot1, &bot2]).await;

        let (bot1, mut bot1_signal) = configure_bot(&bot1).await;

        bot1.register_command(
            "start".to_string(),
            |CommandArgs { room, .. }: CommandArgs| async move {
                room.send(RoomMessageEventContent::text_plain("!hi_bot_2"))
                    .await
                    .expect("Failed to send hi to bot 2");
            },
        )
        .expect("Failed to register command");

        bot1.register_command(
            "hi_bot_1".to_string(),
            |CommandArgs { room, .. }: CommandArgs| async move {
                room.send(RoomMessageEventContent::text_plain("!goodbye_bot_2"))
                    .await
                    .expect("Failed to send goodbye to bot 2");
            },
        )
        .expect("Failed to register command");

        bot1.register_command(
            "goodbye_bot_1".to_string(),
            |CommandArgs { context, .. }: CommandArgs| async move {
                let shutdown = context
                    .get::<Sender<bool>>()
                    .expect("Failed to read from bot context")
                    .expect("Failed to find channel in bot context");
                shutdown
                    .send(true)
                    .await
                    .expect("Failed to send message received signal");
            },
        )
        .expect("Failed to register command");

        let (bot2, mut bot2_signal) = configure_bot(&bot2).await;

        bot2.register_command(
            "start".to_string(),
            |CommandArgs { room, .. }: CommandArgs| async move {
                room.send(RoomMessageEventContent::text_plain("!hi_bot_1"))
                    .await
                    .expect("Failed to send hi to bot 1");
            },
        )
        .expect("Failed to register command");

        bot2.register_command(
            "hi_bot_2".to_string(),
            |CommandArgs { room, .. }: CommandArgs| async move {
                room.send(RoomMessageEventContent::text_plain("!goodbye_bot_1"))
                    .await
                    .expect("Failed to send goodbye to bot 1");
            },
        )
        .expect("Failed to register command");

        bot2.register_command(
            "goodbye_bot_2".to_string(),
            |CommandArgs { context, .. }: CommandArgs| async move {
                let shutdown = context
                    .get::<Sender<bool>>()
                    .expect("Failed to read from bot context")
                    .expect("Failed to find channel in bot context");
                shutdown
                    .send(true)
                    .await
                    .expect("Failed to send message received signal");
            },
        )
        .expect("Failed to register command");

        let (bot1_handle, _) = spawn_bot(bot1).await;
        let (bot2_handle, _) = spawn_bot(bot2).await;

        setup.send_cmd("start", "").await;

        assert!(bot1_signal
            .recv()
            .await
            .expect("Bot1 channel closed unexpectedly"));
        assert!(bot2_signal
            .recv()
            .await
            .expect("Bot2 channel closed unexpectedly"));

        bot1_handle.abort();
        bot2_handle.abort();

        ()
    })
    .await
    .expect("Failed to run bot test in time");
}
