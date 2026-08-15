use std::error::Error;
use std::fmt::{Display, Formatter};
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::conversation::ConversationId;

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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct AgentRun {
    pub(crate) id: AgentRunId,
    pub(crate) conversation_id: ConversationId,
    pub(crate) created_at_milliseconds: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum AgentRunEvent {
    RunStarted {
        request: ModelRequestSnapshot,
    },
    ModelProviderEvent {
        provider: String,
        model: String,
        event_type: String,
        payload: Value,
    },
    RunCompleted {
        response_id: String,
    },
    RunFailed {
        message: String,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct ModelRequestSnapshot {
    pub(crate) provider: String,
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) response_verbosity: Option<ResponseVerbosity>,
    pub(crate) previous_response_id: Option<String>,
    pub(crate) input: Value,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResponseVerbosity {
    Low,
    Medium,
    High,
}

impl ResponseVerbosity {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

impl FromStr for ResponseVerbosity {
    type Err = InvalidResponseVerbosity;

    fn from_str(unvalidated_value: &str) -> Result<Self, Self::Err> {
        match unvalidated_value {
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            _ => Err(InvalidResponseVerbosity),
        }
    }
}

#[derive(Debug)]
pub(crate) struct InvalidResponseVerbosity;

impl Display for InvalidResponseVerbosity {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "verbosity must be low, medium, or high")
    }
}

impl Error for InvalidResponseVerbosity {}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct StoredAgentRunEvent {
    pub(crate) position: u64,
    pub(crate) id: AgentRunEventId,
    pub(crate) timestamp_milliseconds: u64,
    pub(crate) schema_version: u32,
    pub(crate) event: AgentRunEvent,
}
