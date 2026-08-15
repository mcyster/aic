use serde::{Deserialize, Serialize};

use crate::agent_run::{AgentRunEventId, AgentRunId};

use super::ConversationEventId;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ConversationEvent {
    UserPrompt {
        text: String,
    },
    AssistantResponse {
        text: String,
        projection: ProjectionIdentity,
    },
}

impl ConversationEvent {
    pub(crate) fn projection_identity(&self) -> Option<(&'static str, ProjectionIdentity)> {
        match self {
            Self::UserPrompt { .. } => None,
            Self::AssistantResponse { projection, .. } => Some(("assistant", *projection)),
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
