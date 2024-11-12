use matrix_sdk::{Client, Room};
use std::{collections::HashMap, future::Future, pin::Pin};

use crate::{context::MessengerContext, message::Command};

// TODO: To make this more flexible it would be helpful if:
//   1. There was a parser trait that CommandMessageParser implemented
//   2. CommandFn carried an option parser
//   3. MatrixBot carried an optional default parser
//
// This would allow handlers to respond to arbitrary messages instead of
// only messages in a "{cmd} {message}" form

/// The argument struct that is passed to the selected command handler. All watching data and
/// context data is contained in this struct
pub struct CommandArgs {
    pub command: Command,
    pub room: Room,
    pub client: Client,
    pub context: MessengerContext,
}

/// The trait that commands must implement to be able to used as a command handler. This trait
/// generally should not need to be implemented manually. It is blanket implemented for most
/// functions that satisfy the signature async fn(CommandArgs) -> impl Future. The output of the
/// returned future does not matter and is discarded when a handler is run.
///
/// This largely follows the pattern that the Matrix SDK uses for their [EventHandler](matrix_sdk::event_handler::EventHandler).
/// The main difference being that we supply arguments in a single struct.
pub trait CommandFn: Fn(CommandArgs) -> Self::Response + Clone + Send + Sync + 'static {
    type Response: Future + Send + 'static;

    fn run(&self, args: CommandArgs) -> Self::Response;
}

/// Blanket impl for accepting arbitrary functions and closures
impl<T, Resp> CommandFn for T
where
    T: Fn(CommandArgs) -> Resp + Clone + Send + Sync + 'static,
    Resp: Future + Send + 'static,
{
    type Response = Resp;

    fn run(&self, args: CommandArgs) -> Self::Response {
        (self)(args)
    }
}

// Internal bot type for storing a command handler function
pub type CommandHandler =
    Box<dyn Fn(CommandArgs) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;
pub type CommandHandlers = HashMap<String, CommandHandler>;
