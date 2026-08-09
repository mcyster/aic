use std::error::Error;
use std::fmt::{Display, Formatter};
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct ConversationId(Uuid);

impl ConversationId {
    pub(crate) fn new() -> Self {
        Self(Uuid::now_v7())
    }

    pub(crate) fn storage_key(self) -> String {
        self.0.simple().to_string()
    }
}

impl Display for ConversationId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "conversation_{}", self.0.simple())
    }
}

impl FromStr for ConversationId {
    type Err = InvalidIdentifier;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let uuid_text = text.strip_prefix("conversation_").unwrap_or(text);
        Uuid::parse_str(uuid_text)
            .map(Self)
            .map_err(InvalidIdentifier)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct ConversationEventId(Uuid);

impl ConversationEventId {
    pub(crate) fn new() -> Self {
        Self(Uuid::now_v7())
    }

    pub(crate) fn storage_key(self) -> String {
        self.0.simple().to_string()
    }
}

impl Display for ConversationEventId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "conversation_event_{}", self.0.simple())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct AgentRunId(Uuid);

impl AgentRunId {
    pub(crate) fn new() -> Self {
        Self(Uuid::now_v7())
    }

    pub(crate) fn storage_key(self) -> String {
        self.0.simple().to_string()
    }
}

impl Display for AgentRunId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "agent_run_{}", self.0.simple())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct AgentRunEventId(Uuid);

impl AgentRunEventId {
    pub(crate) fn new() -> Self {
        Self(Uuid::now_v7())
    }

    pub(crate) fn storage_key(self) -> String {
        self.0.simple().to_string()
    }
}

impl Display for AgentRunEventId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "agent_run_event_{}", self.0.simple())
    }
}

#[derive(Debug)]
pub(crate) struct InvalidIdentifier(uuid::Error);

impl Display for InvalidIdentifier {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "invalid identifier: {}", self.0)
    }
}

impl Error for InvalidIdentifier {}
