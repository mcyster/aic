use std::error::Error;
use std::fmt::{Display, Formatter};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use super::{
    ConversationEventId, ConversationId, ConversationProblem, InvalidConversationProblem,
    InvalidModelData, ModelDetails,
};

pub(crate) const SCHEMA_VERSION: u32 = 10;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct ConversationEvent {
    pub(crate) conversation_id: ConversationId,
    pub(crate) position: u64,
    pub(crate) id: ConversationEventId,
    #[serde(with = "time::serde::rfc3339")]
    pub(crate) timestamp: OffsetDateTime,
    pub(crate) schema_version: u32,
    #[serde(flatten)]
    pub(crate) kind: ConversationEventKind,
}

impl ConversationEvent {
    pub(crate) fn new(
        conversation_id: ConversationId,
        position: u64,
        kind: ConversationEventKind,
    ) -> Self {
        Self {
            conversation_id,
            position,
            id: ConversationEventId::new(),
            timestamp: OffsetDateTime::now_utc(),
            schema_version: SCHEMA_VERSION,
            kind,
        }
    }

    pub(super) fn ensure_valid(&self) -> Result<(), InvalidConversationEventKind> {
        self.kind.ensure_valid()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ConversationEventKind {
    User {
        content: Vec<UserContent>,
    },
    Model {
        model: ModelDetails,
        event: ModelEvent,
    },
    Problem {
        model: Option<ModelDetails>,
        problem: ConversationProblem,
    },
}

impl ConversationEventKind {
    pub(super) fn ensure_valid(&self) -> Result<(), InvalidConversationEventKind> {
        match self {
            Self::User { .. } => Ok(()),
            Self::Model { model, event } => {
                model
                    .ensure_valid()
                    .map_err(InvalidConversationEventKind::ModelData)?;
                event
                    .ensure_valid()
                    .map_err(InvalidConversationEventKind::ModelEvent)
            }
            Self::Problem { model, problem } => {
                if let Some(model) = model {
                    model
                        .ensure_valid()
                        .map_err(InvalidConversationEventKind::ModelData)?;
                }
                problem
                    .ensure_valid()
                    .map_err(InvalidConversationEventKind::ConversationProblem)
            }
        }
    }
}

#[derive(Debug)]
pub(super) enum InvalidConversationEventKind {
    ModelEvent(InvalidModelEvent),
    ConversationProblem(InvalidConversationProblem),
    ModelData(InvalidModelData),
}

impl Display for InvalidConversationEventKind {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ModelEvent(error) => Display::fmt(error, formatter),
            Self::ConversationProblem(error) => Display::fmt(error, formatter),
            Self::ModelData(error) => Display::fmt(error, formatter),
        }
    }
}

impl Error for InvalidConversationEventKind {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ModelEvent {
    Assistant(AssistantResponse),
    Communication(ModelCommunication),
}

impl ModelEvent {
    pub(crate) fn message(&self) -> &str {
        match self {
            Self::Assistant(response) => response.message(),
            Self::Communication(communication) => communication.message(),
        }
    }

    pub(crate) fn importance(&self) -> ModelEventImportance {
        match self {
            Self::Assistant(_) => ModelEventImportance::Important,
            Self::Communication(communication) => communication.importance(),
        }
    }

    fn ensure_valid(&self) -> Result<(), InvalidModelEvent> {
        match self {
            Self::Assistant(response) => response
                .ensure_valid()
                .map_err(InvalidModelEvent::Assistant),
            Self::Communication(communication) => communication
                .ensure_valid()
                .map_err(InvalidModelEvent::ModelCommunication),
        }
    }
}

#[derive(Debug)]
pub(super) enum InvalidModelEvent {
    Assistant(InvalidAssistantResponse),
    ModelCommunication(InvalidModelCommunication),
}

impl Display for InvalidModelEvent {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Assistant(error) => Display::fmt(error, formatter),
            Self::ModelCommunication(error) => Display::fmt(error, formatter),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct AssistantResponse {
    message: String,
}

impl AssistantResponse {
    pub(crate) fn new(message: String) -> Result<Self, InvalidAssistantResponse> {
        let response = Self { message };
        response.ensure_valid()?;
        Ok(response)
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    fn ensure_valid(&self) -> Result<(), InvalidAssistantResponse> {
        if self.message.trim().is_empty() {
            return Err(InvalidAssistantResponse::EmptyMessage);
        }
        Ok(())
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum InvalidAssistantResponse {
    EmptyMessage,
}

impl Display for InvalidAssistantResponse {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyMessage => write!(formatter, "assistant response message must not be empty"),
        }
    }
}

impl Error for InvalidAssistantResponse {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct ModelCommunication {
    message: String,
    importance: ModelEventImportance,
    subtype: String,
}

impl ModelCommunication {
    pub(crate) fn new(
        message: String,
        importance: ModelEventImportance,
        subtype: String,
    ) -> Result<Self, InvalidModelCommunication> {
        let communication = Self {
            message,
            importance,
            subtype,
        };
        communication.ensure_valid()?;
        Ok(communication)
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn importance(&self) -> ModelEventImportance {
        self.importance
    }

    pub(crate) fn subtype(&self) -> &str {
        &self.subtype
    }

    fn ensure_valid(&self) -> Result<(), InvalidModelCommunication> {
        if self.message.trim().is_empty() {
            return Err(InvalidModelCommunication::EmptyMessage);
        }
        if self.subtype().trim().is_empty() {
            return Err(InvalidModelCommunication::EmptySubtype);
        }
        Ok(())
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum InvalidModelCommunication {
    EmptyMessage,
    EmptySubtype,
}

impl Display for InvalidModelCommunication {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyMessage => {
                write!(formatter, "model communication message must not be empty")
            }
            Self::EmptySubtype => {
                write!(formatter, "model communication subtype must not be empty")
            }
        }
    }
}

impl Error for InvalidModelCommunication {}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ModelEventImportance {
    Detailed,
    Interesting,
    Important,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub(crate) enum UserContent {
    Text(String),
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use serde_json::{Map, Value, json};
    use time::{Date, Month, OffsetDateTime};

    use super::{
        AssistantResponse, ConversationEvent, ConversationEventKind, InvalidAssistantResponse,
        InvalidModelCommunication, ModelCommunication, ModelEvent, ModelEventImportance,
    };
    use crate::conversation::{
        ConversationEventId, ConversationId, ConversationProblem, ModelData, ModelDetails, ModelId,
        ModelIssue, ModelSource, ProviderId, UserContent,
    };

    fn model_content() -> Map<String, Value> {
        Map::from_iter([("response_id".to_owned(), Value::String("resp_1".to_owned()))])
    }

    fn model_data() -> ModelData {
        ModelData::new(model_content()).expect("the model data should be valid")
    }

    fn source() -> ModelSource {
        ModelSource::new(
            ProviderId::from_str("openai").expect("the provider identifier should be valid"),
            ModelId::from_str("gpt-5.6").expect("the model identifier should be valid"),
        )
    }

    fn fixed_timestamp() -> OffsetDateTime {
        Date::from_calendar_date(2026, Month::August, 22)
            .expect("the date should be valid")
            .with_hms_milli(18, 42, 31, 482)
            .expect("the time should be valid")
            .assume_utc()
    }

    #[test]
    fn canonical_user_event_round_trips_with_flattened_kind() {
        let conversation_id = ConversationId::new();
        let event_id = ConversationEventId::new();
        let conversation_event = ConversationEvent {
            conversation_id,
            position: 0,
            id: event_id,
            timestamp: fixed_timestamp(),
            schema_version: 7,
            kind: ConversationEventKind::User {
                content: vec![UserContent::Text("Hello".to_owned())],
            },
        };

        let json = serde_json::to_value(&conversation_event)
            .expect("the conversation event should serialize");
        let deserialized_event: ConversationEvent = serde_json::from_value(json.clone())
            .expect("the conversation event should deserialize");

        assert_eq!(
            json,
            json!({
                "conversation_id": conversation_id,
                "position": 0,
                "id": event_id,
                "timestamp": "2026-08-22T18:42:31.482Z",
                "schema_version": 7,
                "type": "user",
                "content": [{ "type": "text", "value": "Hello" }]
            })
        );
        assert_eq!(deserialized_event, conversation_event);
    }

    #[test]
    fn model_event_round_trips_with_model_details() {
        let conversation_id = ConversationId::new();
        let event_id = ConversationEventId::new();
        let conversation_event = ConversationEvent {
            conversation_id,
            position: 1,
            id: event_id,
            timestamp: fixed_timestamp(),
            schema_version: 7,
            kind: ConversationEventKind::Model {
                model: ModelDetails::new(source(), Some(model_data()))
                    .expect("the model details should be valid"),
                event: ModelEvent::Assistant(
                    AssistantResponse::new("The answer is 42.".to_owned())
                        .expect("the assistant response should be valid"),
                ),
            },
        };

        let json = serde_json::to_value(&conversation_event)
            .expect("the conversation event should serialize");
        let deserialized_event: ConversationEvent = serde_json::from_value(json.clone())
            .expect("the conversation event should deserialize");

        assert_eq!(
            json,
            json!({
                "conversation_id": conversation_id,
                "position": 1,
                "id": event_id,
                "timestamp": "2026-08-22T18:42:31.482Z",
                "schema_version": 7,
                "type": "model",
                "model": {
                    "source": { "provider": "openai", "model": "gpt-5.6" },
                    "data": { "response_id": "resp_1" }
                },
                "event": {
                    "type": "assistant",
                    "message": "The answer is 42."
                }
            })
        );
        assert_eq!(deserialized_event, conversation_event);
    }

    #[test]
    fn assistant_response_round_trips_through_json() {
        let event = ModelEvent::Assistant(
            AssistantResponse::new("The answer is 42.".to_owned())
                .expect("the assistant response should be valid"),
        );

        let json = serde_json::to_value(&event).expect("the assistant response should serialize");
        let deserialized_event: ModelEvent = serde_json::from_value(json.clone())
            .expect("the assistant response should deserialize");

        assert_eq!(
            json,
            json!({
                "type": "assistant",
                "message": "The answer is 42."
            })
        );
        assert_eq!(deserialized_event, event);
        assert_eq!(event.message(), "The answer is 42.");
        assert_eq!(event.importance(), ModelEventImportance::Important);
    }

    #[test]
    fn assistant_response_rejects_an_empty_message() {
        assert_eq!(
            AssistantResponse::new(String::new()),
            Err(InvalidAssistantResponse::EmptyMessage)
        );
    }

    #[test]
    fn conversation_problem_uses_one_authoritative_conversation_event_surface() {
        let event = ConversationEventKind::Problem {
            model: Some(
                ModelDetails::new(source(), None).expect("the model details should be valid"),
            ),
            problem: ConversationProblem::Issue(
                ModelIssue::try_refusal("I cannot comply.".to_owned())
                    .expect("the refusal should be valid"),
            ),
        };

        let serialized = serde_json::to_value(&event).expect("the problem should serialize");
        let deserialized: ConversationEventKind =
            serde_json::from_value(serialized.clone()).expect("the problem should deserialize");

        assert_eq!(
            serialized,
            json!({
                "type": "problem",
                "model": {
                    "source": {
                        "provider": "openai",
                        "model": "gpt-5.6"
                    }
                },
                "problem": {
                    "category": "issue",
                    "detail": {
                        "type": "refusal",
                        "message": "I cannot comply."
                    }
                }
            })
        );
        assert_eq!(deserialized, event);
        assert!(serialized.get("message").is_none());
        assert!(serialized.get("severity").is_none());
    }

    #[test]
    fn assistant_response_rejects_a_whitespace_only_message() {
        assert_eq!(
            AssistantResponse::new(" \n\t ".to_owned()),
            Err(InvalidAssistantResponse::EmptyMessage)
        );
    }

    #[test]
    fn assistant_response_preserves_surrounding_whitespace() {
        let response = AssistantResponse::new("  answer\n".to_owned())
            .expect("the assistant response should be valid");

        assert_eq!(response.message(), "  answer\n");
    }

    #[test]
    fn model_communication_round_trips_through_json() {
        let event = ModelEvent::Communication(
            ModelCommunication::new(
                "I compared the two approaches.".to_owned(),
                ModelEventImportance::Detailed,
                "reasoning".to_owned(),
            )
            .expect("the model communication should be valid"),
        );

        let json = serde_json::to_value(&event).expect("the communication should serialize");
        let deserialized_event: ModelEvent =
            serde_json::from_value(json.clone()).expect("the communication should deserialize");

        assert_eq!(
            json,
            json!({
                "type": "communication",
                "message": "I compared the two approaches.",
                "importance": "detailed",
                "subtype": "reasoning"
            })
        );
        assert_eq!(deserialized_event, event);
        assert_eq!(event.message(), "I compared the two approaches.");
        assert_eq!(event.importance(), ModelEventImportance::Detailed);
        let ModelEvent::Communication(communication) = &event else {
            panic!("the event should be a model communication");
        };
        assert_eq!(communication.subtype(), "reasoning");
    }

    #[test]
    fn model_communication_rejects_an_empty_message() {
        assert_eq!(
            ModelCommunication::new(
                String::new(),
                ModelEventImportance::Detailed,
                "reasoning".to_owned(),
            ),
            Err(InvalidModelCommunication::EmptyMessage)
        );
        assert_eq!(
            ModelCommunication::new(
                " \n ".to_owned(),
                ModelEventImportance::Detailed,
                "reasoning".to_owned(),
            ),
            Err(InvalidModelCommunication::EmptyMessage)
        );
    }

    #[test]
    fn model_communication_rejects_an_empty_subtype() {
        assert_eq!(
            ModelCommunication::new(
                "message".to_owned(),
                ModelEventImportance::Detailed,
                " \t ".to_owned(),
            ),
            Err(InvalidModelCommunication::EmptySubtype)
        );
    }

    #[test]
    fn model_communication_preserves_its_message_and_subtype() {
        let communication = ModelCommunication::new(
            "  message\n".to_owned(),
            ModelEventImportance::Interesting,
            " reasoning_summary ".to_owned(),
        )
        .expect("the model communication should be valid");

        assert_eq!(communication.message(), "  message\n");
        assert_eq!(communication.subtype(), " reasoning_summary ");
    }

    #[test]
    fn canonical_model_event_round_trip_preserves_source_and_metadata() {
        let conversation_id = ConversationId::new();
        let event_id = ConversationEventId::new();
        let source = source();
        let model_event = ModelEvent::Assistant(
            AssistantResponse::new("The answer is 42.".to_owned())
                .expect("the assistant response should be valid"),
        );
        let conversation_event = ConversationEvent {
            conversation_id,
            position: 1,
            id: event_id,
            timestamp: fixed_timestamp(),
            schema_version: 7,
            kind: ConversationEventKind::Model {
                model: ModelDetails::new(source.clone(), None)
                    .expect("the model details should be valid"),
                event: model_event.clone(),
            },
        };

        let json = serde_json::to_value(&conversation_event)
            .expect("the conversation event should serialize");
        let deserialized_event: ConversationEvent = serde_json::from_value(json.clone())
            .expect("the conversation event should deserialize");

        assert_eq!(
            json,
            json!({
                "conversation_id": conversation_id,
                "position": 1,
                "id": event_id,
                "timestamp": "2026-08-22T18:42:31.482Z",
                "schema_version": 7,
                "type": "model",
                "model": {
                    "source": { "provider": "openai", "model": "gpt-5.6" }
                },
                "event": {
                    "type": "assistant",
                    "message": "The answer is 42."
                }
            })
        );
        assert_eq!(deserialized_event, conversation_event);
        assert_eq!(
            conversation_event.kind,
            ConversationEventKind::Model {
                model: ModelDetails::new(source, None).expect("the model details should be valid"),
                event: model_event,
            }
        );
    }
}
