use microbot::{CommandArgs, MatrixMessenger};
use rand::{distributions::Alphanumeric, Rng};
use tokio::sync::{mpsc, mpsc::Sender};
use tracing_subscriber::filter::EnvFilter;

use matrix_chat_bot_test_utils::{setup, spawn_bot};

static HOMESERVER: &'static str = "http://localhost:8008";

#[tokio::test]
async fn test_receives_command() {
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

        let (tx, mut rx) = mpsc::channel::<bool>(1);

        let mut bot = MatrixMessenger::new(HOMESERVER)
            .await
            .expect("Failed to create chat bot");
        bot.set_prefix(Some("!".to_string()))
            .expect("Failed to set command prefix");

        bot.login(&receiver, &receiver)
            .await
            .expect("Failed to login to homeserver");

        bot.insert_data(tx);

        bot.register_command(
            "test_cmd".to_string(),
            |args: CommandArgs| async {
                let CommandArgs {
                    command, context, ..
                } = args;

                if command.message == "test_receives_message" {
                    let sender = context
                        .get::<Sender<bool>>()
                        .expect("Failed to find channel in bot context");
                    sender
                        .send(true)
                        .await
                        .expect("Failed to send message received signal");
                }
            },
        );

        let (bot_handle, _) = spawn_bot(bot).await;

        setup.send_cmd("test_cmd", "test_receives_message").await;

        assert!(rx.recv().await.expect("Channel closed unexpectedly"));

        bot_handle.abort();
    })
    .await
    .expect("Failed to run bot test in time");
}
