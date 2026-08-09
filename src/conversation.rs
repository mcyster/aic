use std::error::Error;
use std::fmt::{Display, Formatter};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::identifier::{AgentRunEventId, AgentRunId, ConversationEventId, ConversationId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UserPrompt(String);

impl UserPrompt {
    pub(crate) fn text(&self) -> &str {
        &self.0
    }
}

impl FromStr for UserPrompt {
    type Err = InvalidUserPrompt;

    fn from_str(unvalidated_text: &str) -> Result<Self, Self::Err> {
        let normalized_text = unvalidated_text.trim();

        if normalized_text.is_empty() {
            return Err(InvalidUserPrompt);
        }

        Ok(Self(normalized_text.to_owned()))
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct InvalidUserPrompt;

impl Display for InvalidUserPrompt {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "user prompt must not be empty")
    }
}

impl Error for InvalidUserPrompt {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct Conversation {
    pub(crate) id: ConversationId,
    pub(crate) created_at_milliseconds: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ConversationEvent {
    User {
        text: String,
    },
    Assistant {
        text: String,
        projection: ProjectionIdentity,
    },
}

impl ConversationEvent {
    pub(crate) fn projection_identity(&self) -> Option<(&'static str, ProjectionIdentity)> {
        match self {
            Self::User { .. } => None,
            Self::Assistant { projection, .. } => Some(("assistant", *projection)),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct ProjectionIdentity {
    pub(crate) source_run_id: AgentRunId,
    pub(crate) source_run_event_id: AgentRunEventId,
    pub(crate) output_index: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct StoredConversationEvent {
    pub(crate) position: u64,
    pub(crate) id: ConversationEventId,
    pub(crate) timestamp_milliseconds: u64,
    pub(crate) schema_version: u32,
    pub(crate) event: ConversationEvent,
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::{InvalidUserPrompt, UserPrompt};

    #[test]
    fn user_prompt_rejects_empty_text() {
        let parsing_result = UserPrompt::from_str("   ");

        assert_eq!(parsing_result, Err(InvalidUserPrompt));
    }

    #[test]
    fn user_prompt_normalizes_surrounding_whitespace() {
        let user_prompt = UserPrompt::from_str("  explain Rust ownership  ")
            .expect("the user prompt should be valid");

        assert_eq!(user_prompt.text(), "explain Rust ownership");
    }
}
