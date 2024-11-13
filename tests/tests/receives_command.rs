// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use microbot::{CommandArgs, MatrixConfig, MatrixMessenger};
use microbot_test_utils::{setup, spawn_bot};
use rand::{distributions::Alphanumeric, Rng};
use tokio::sync::{mpsc, mpsc::Sender};
use tracing_subscriber::filter::EnvFilter;

static HOMESERVER: &'static str = "http://localhost:8008";

#[tokio::test]
async fn test_receives_command() {
    tracing_subscriber::fmt()
        .pretty()
        .with_env_filter(EnvFilter::from_default_env())
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
    let receiver = format!(
        "test-receiver-{}",
        rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(24)
            .map(|b| char::from(b))
            .collect::<String>()
    );

    // This timeout ensures that the test fails if communication to the bot does not succeed or
    // if connections to the server hang
    tokio::time::timeout(tokio::time::Duration::from_secs(30), async {
        let setup = setup(HOMESERVER, &sender, &[&receiver]).await;

        let (tx, mut rx) = mpsc::channel::<bool>(1);

        let bot = MatrixMessenger::new(MatrixConfig {
            url: HOMESERVER.to_string(),
            user: receiver.to_string(),
            password: receiver.to_string(),
            display_name: receiver.to_string(),
            command_prefix: Some("!".to_string()),
        });

        bot.insert_data(tx).expect("Failed to add bot context data");

        bot.register_command("test_cmd".to_string(), |args: CommandArgs| async {
            let CommandArgs {
                command, context, ..
            } = args;

            if command.message == "test_receives_message" {
                let sender = context
                    .get::<Sender<bool>>()
                    .expect("Failed to read from bot context")
                    .expect("Failed to find channel in bot context");
                sender
                    .send(true)
                    .await
                    .expect("Failed to send message received signal");
            }
        })
        .expect("Failed to register command");

        let (bot_handle, _) = spawn_bot(bot).await;

        setup.send_cmd("test_cmd", "test_receives_message").await;

        assert!(rx.recv().await.expect("Channel closed unexpectedly"));

        bot_handle.abort();
    })
    .await
    .expect("Failed to run bot test in time");
}
