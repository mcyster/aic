use std::error::Error;
use std::fmt::{Display, Formatter};

use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;

use crate::conversation::{Conversation, ModelEvent, ModelIssue, ModelSource};

pub(crate) type ModelOutputStream = BoxStream<'static, Result<ModelDriverOutput, ModelDriverError>>;

pub(crate) enum ModelDriverOutput {
    Event(ModelEvent),
    Issue(ModelIssue),
}

pub(crate) trait ModelDriver {
    fn source(&self) -> &ModelSource;

    fn invoke<'invoke>(
        &'invoke self,
        conversation: &'invoke Conversation,
    ) -> BoxFuture<'invoke, Result<ModelOutputStream, ModelDriverError>>;
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ModelDriverError {
    Authentication(String),
    RateLimited(String),
    Transport(String),
    InvalidRequest(String),
    InvalidResponse(String),
    StreamInterrupted(String),
    Provider(String),
}

impl Display for ModelDriverError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Authentication(message) => write!(formatter, "authentication failed: {message}"),
            Self::RateLimited(message) => write!(formatter, "rate limited: {message}"),
            Self::Transport(message) => write!(formatter, "model transport failed: {message}"),
            Self::InvalidRequest(message) => write!(formatter, "invalid model request: {message}"),
            Self::InvalidResponse(message) => {
                write!(formatter, "invalid model response: {message}")
            }
            Self::StreamInterrupted(message) => {
                write!(
                    formatter,
                    "model response stream was interrupted: {message}"
                )
            }
            Self::Provider(message) => write!(formatter, "model provider failed: {message}"),
        }
    }
}

impl Error for ModelDriverError {}
