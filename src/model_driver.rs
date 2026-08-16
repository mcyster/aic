use std::error::Error;
use std::fmt::{Display, Formatter};
use std::str::FromStr;

use crate::conversation::ConversationEvent;

pub(crate) trait ModelDriver {
    fn model(&self) -> &ModelId;

    fn invoke(
        &self,
        conversation: &[ConversationEvent],
    ) -> Result<Vec<ConversationEvent>, ModelDriverError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ModelId(String);

impl ModelId {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ModelId {
    type Err = InvalidModelId;

    fn from_str(unvalidated_value: &str) -> Result<Self, Self::Err> {
        let normalized_value = unvalidated_value.trim();
        if normalized_value.is_empty() {
            return Err(InvalidModelId);
        }
        Ok(Self(normalized_value.to_owned()))
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct InvalidModelId;

impl Display for InvalidModelId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "model identifier must not be empty")
    }
}

impl Error for InvalidModelId {}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ModelDriverError {
    Authentication(String),
    RateLimited(String),
    Transport(String),
    InvalidRequest(String),
    InvalidResponse(String),
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
            Self::Provider(message) => write!(formatter, "model provider failed: {message}"),
        }
    }
}

impl Error for ModelDriverError {}
