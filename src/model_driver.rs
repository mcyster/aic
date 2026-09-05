use std::error::Error;
use std::fmt::{Display, Formatter};

use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;

use crate::conversation::{
    Conversation, ConversationEvent, ConversationTurnId, DriverEventReader, ModelSource,
};

pub(crate) type ModelOutputStream = BoxStream<'static, Result<ConversationEvent, ModelDriverError>>;

pub(crate) trait ModelDriver: DriverEventReader {
    fn source(&self) -> &ModelSource;

    fn invoke<'invoke>(
        &'invoke self,
        conversation: &'invoke Conversation,
        turn_id: ConversationTurnId,
    ) -> BoxFuture<'invoke, Result<ModelOutputStream, ModelDriverError>>;
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ModelDriverError {
    InvalidOutput(String),
    IncompleteTurn,
}

impl Display for ModelDriverError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidOutput(message) => {
                write!(formatter, "invalid model driver output: {message}")
            }
            Self::IncompleteTurn => write!(
                formatter,
                "the model driver ended without completing the turn"
            ),
        }
    }
}

impl Error for ModelDriverError {}
