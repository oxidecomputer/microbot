// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use matrix_chat_bot::{CommandArgs, MatrixBot};
use rand::{distributions::Alphanumeric, Rng};
use tokio::sync::{mpsc, mpsc::Sender};
use tracing_subscriber::filter::EnvFilter;

use matrix_chat_bot_test_utils::{setup, spawn_bot};

static HOMESERVER: &'static str = "http://localhost:8008";

#[tokio::test]
async fn test_ignores_old_messages() {
    tracing_subscriber::fmt()
        .pretty()
        .with_test_writer()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let sender = format!(
        "test-sender-{}",
        rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(24)
            .collect::<String>()
    );
    let receiver = format!(
        "test-receiver-{}",
        rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(24)
            .collect::<String>()
    );

    // This timeout ensures that the test fails if communication to the bot does not succeed or
    // if connections to the server hang
    tokio::time::timeout(tokio::time::Duration::from_secs(30), async {
        let setup = setup(HOMESERVER, &sender, &[&receiver]).await;

        // This command is send to the room prior to the bot joining, and should therefore NOT
        // be responded to by the bot
        setup.send_cmd("test_cmd1", "").await;

        let (tx, mut rx) = mpsc::channel::<u8>(1);

        let mut bot = MatrixBot::new(HOMESERVER)
            .await
            .expect("Failed to create chat bot");
        bot.set_prefix(Some("!".to_string()))
            .expect("Failed to set command prefix");

        bot.login(&receiver, &receiver)
            .await
            .expect("Failed to login to homeserver");

        bot.insert_data(tx);

        bot.register_command(
            "test_cmd1".to_string(),
            |CommandArgs { context, .. }: CommandArgs| async move {
                context
                    .get::<Sender<u8>>()
                    .expect("Failed to find channel in bot context for cmd1")
                    .send(1)
                    .await
                    .expect("Failed to send message received signal from cmd1");
            },
        );

        bot.register_command(
            "test_cmd2".to_string(),
            |CommandArgs { context, .. }: CommandArgs| async move {
                println!("Run test_cmd2 handler");
                context
                    .get::<Sender<u8>>()
                    .expect("Failed to find channel in bot context for cmd2")
                    .send(2)
                    .await
                    .expect("Failed to send message received signal from cmd2");
            },
        );

        let (bot_handle, _) = spawn_bot(bot).await;

        setup.send_cmd("test_cmd2", "").await;

        assert_eq!(
            2,
            rx.recv().await.expect("Channel closed unexpectedly"),
            "Check that value received came from test_cmd2 and not test_cmd1"
        );

        bot_handle.abort();
    })
    .await
    .expect("Failed to run bot test in time");
}
