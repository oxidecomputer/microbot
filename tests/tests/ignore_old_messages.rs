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

        // This command is send to the room prior to the bot joining, and should therefore NOT
        // be responded to by the bot
        setup.send_cmd("test_cmd1", "").await;

        let (tx, mut rx) = mpsc::channel::<u8>(1);

        let mut bot = MatrixMessenger::new(MatrixConfig {
            url: HOMESERVER.to_string(),
            user: receiver.to_string(),
            password: receiver.to_string(),
            display_name: receiver.to_string(),
            command_prefix: Some("!".to_string()),
        });

        bot.insert_data(tx).expect("Failed to add bot context data");

        bot.register_command(
            "test_cmd1".to_string(),
            |CommandArgs {
                 command, context, ..
             }: CommandArgs| async move {
                tracing::info!(?command, "Handling test_cmd1");
                context
                    .get::<Sender<u8>>()
                    .expect("Failed to read from bot context")
                    .expect("Failed to find channel in bot context for cmd1")
                    .send(1)
                    .await
                    .expect("Failed to send message received signal from cmd1");
            },
        )
        .expect("Failed to register command");

        bot.register_command(
            "test_cmd2".to_string(),
            |CommandArgs {
                 command, context, ..
             }: CommandArgs| async move {
                tracing::info!(?command, "Handling test_cmd2");
                context
                    .get::<Sender<u8>>()
                    .expect("Failed to read from bot context")
                    .expect("Failed to find channel in bot context for cmd2")
                    .send(2)
                    .await
                    .expect("Failed to send message received signal from cmd2");
            },
        )
        .expect("Failed to register command");

        let _ = spawn_bot(&mut bot).await;

        setup.send_cmd("test_cmd2", "").await;

        assert_eq!(
            2,
            rx.recv().await.expect("Channel closed unexpectedly"),
            "Check that value received came from test_cmd2 and not test_cmd1"
        );

        bot.abort().expect("Failed to stop bot");
    })
    .await
    .expect("Failed to run bot test in time");
}
