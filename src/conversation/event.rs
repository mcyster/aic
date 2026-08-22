use std::error::Error;
use std::fmt::{Display, Formatter};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use time::OffsetDateTime;

use super::{ConversationEventId, ConversationId, ModelSource};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ConversationEvent {
    User {
        content: Vec<UserContent>,
    },
    Model {
        source: ModelSource,
        event: ModelEvent,
    },
}

impl ConversationEvent {
    pub(crate) fn from_model_event(source: ModelSource, event: ModelEvent) -> Self {
        Self::Model { source, event }
    }

    pub(super) fn ensure_valid(&self) -> Result<(), InvalidConversationEvent> {
        match self {
            Self::User { .. } => Ok(()),
            Self::Model { event, .. } => event
                .ensure_valid()
                .map_err(InvalidConversationEvent::InvalidModelEvent),
        }
    }
}

#[derive(Debug)]
pub(super) enum InvalidConversationEvent {
    InvalidModelEvent(InvalidModelEvent),
}

impl Display for InvalidConversationEvent {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidModelEvent(error) => Display::fmt(error, formatter),
        }
    }
}

impl Error for InvalidConversationEvent {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ModelEvent {
    AssistantResponse(AssistantResponse),
    Communication(ModelCommunication),
}

impl ModelEvent {
    pub(crate) fn message(&self) -> &str {
        match self {
            Self::AssistantResponse(response) => response.message(),
            Self::Communication(communication) => communication.message(),
        }
    }

    pub(crate) fn importance(&self) -> ModelEventImportance {
        match self {
            Self::AssistantResponse(_) => ModelEventImportance::Important,
            Self::Communication(communication) => communication.importance(),
        }
    }

    fn ensure_valid(&self) -> Result<(), InvalidModelEvent> {
        match self {
            Self::AssistantResponse(response) => response
                .ensure_valid()
                .map_err(InvalidModelEvent::InvalidAssistantResponse),
            Self::Communication(communication) => communication
                .ensure_valid()
                .map_err(InvalidModelEvent::InvalidModelCommunication),
        }
    }
}

#[derive(Debug)]
pub(super) enum InvalidModelEvent {
    InvalidAssistantResponse(InvalidAssistantResponse),
    InvalidModelCommunication(InvalidModelCommunication),
}

impl Display for InvalidModelEvent {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidAssistantResponse(error) => Display::fmt(error, formatter),
            Self::InvalidModelCommunication(error) => Display::fmt(error, formatter),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct AssistantResponse {
    message: String,
    extensions: Map<String, Value>,
}

impl AssistantResponse {
    pub(crate) fn new(
        message: String,
        extensions: Map<String, Value>,
    ) -> Result<Self, InvalidAssistantResponse> {
        let response = Self {
            message,
            extensions,
        };
        response.ensure_valid()?;
        Ok(response)
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    #[allow(dead_code)]
    pub(crate) fn extensions(&self) -> &Map<String, Value> {
        &self.extensions
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
    extensions: Map<String, Value>,
}

impl ModelCommunication {
    pub(crate) fn new(
        message: String,
        importance: ModelEventImportance,
        subtype: String,
        extensions: Map<String, Value>,
    ) -> Result<Self, InvalidModelCommunication> {
        let communication = Self {
            message,
            importance,
            subtype,
            extensions,
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

    #[allow(dead_code)]
    pub(crate) fn extensions(&self) -> &Map<String, Value> {
        &self.extensions
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct StoredConversationEvent {
    pub(crate) conversation_id: ConversationId,
    pub(crate) position: u64,
    pub(crate) id: ConversationEventId,
    #[serde(with = "time::serde::rfc3339")]
    pub(crate) timestamp: OffsetDateTime,
    pub(crate) schema_version: u32,
    pub(crate) event: ConversationEvent,
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use serde_json::{Map, Value, json};
    use time::{Date, Month, OffsetDateTime};

    use super::{
        AssistantResponse, ConversationEvent, InvalidAssistantResponse, InvalidModelCommunication,
        ModelCommunication, ModelEvent, ModelEventImportance, StoredConversationEvent,
    };
    use crate::conversation::{
        ConversationEventId, ConversationId, ModelId, ModelSource, ProviderId, UserContent,
    };

    fn extensions() -> Map<String, Value> {
        Map::from_iter([(
            "openai.item_id".to_owned(),
            Value::String("item_1".to_owned()),
        )])
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
    fn stored_conversation_event_timestamp_round_trips_as_rfc3339() {
        let stored_event = StoredConversationEvent {
            conversation_id: ConversationId::new(),
            position: 0,
            id: ConversationEventId::new(),
            timestamp: fixed_timestamp(),
            schema_version: 6,
            event: ConversationEvent::User {
                content: vec![UserContent::Text("hello".to_owned())],
            },
        };

        let json = serde_json::to_value(&stored_event).expect("the stored event should serialize");
        let deserialized_event: StoredConversationEvent =
            serde_json::from_value(json.clone()).expect("the stored event should deserialize");

        assert_eq!(json["timestamp"], json!("2026-08-22T18:42:31.482Z"));
        assert_eq!(deserialized_event, stored_event);
    }

    #[test]
    fn assistant_response_round_trips_through_json() {
        let event = ModelEvent::AssistantResponse(
            AssistantResponse::new("The answer is 42.".to_owned(), extensions())
                .expect("the assistant response should be valid"),
        );

        let json = serde_json::to_value(&event).expect("the assistant response should serialize");
        let deserialized_event: ModelEvent = serde_json::from_value(json.clone())
            .expect("the assistant response should deserialize");

        assert_eq!(
            json,
            json!({
                "type": "assistant_response",
                "message": "The answer is 42.",
                "extensions": { "openai.item_id": "item_1" }
            })
        );
        assert_eq!(deserialized_event, event);
        assert_eq!(event.message(), "The answer is 42.");
        assert_eq!(event.importance(), ModelEventImportance::Important);
        let ModelEvent::AssistantResponse(response) = &event else {
            panic!("the event should be an assistant response");
        };
        assert_eq!(response.extensions(), &extensions());
    }

    #[test]
    fn assistant_response_rejects_an_empty_message() {
        assert_eq!(
            AssistantResponse::new(String::new(), Map::new()),
            Err(InvalidAssistantResponse::EmptyMessage)
        );
    }

    #[test]
    fn assistant_response_rejects_a_whitespace_only_message() {
        assert_eq!(
            AssistantResponse::new(" \n\t ".to_owned(), Map::new()),
            Err(InvalidAssistantResponse::EmptyMessage)
        );
    }

    #[test]
    fn assistant_response_preserves_surrounding_whitespace() {
        let response = AssistantResponse::new("  answer\n".to_owned(), Map::new())
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
                extensions(),
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
                "subtype": "reasoning",
                "extensions": { "openai.item_id": "item_1" }
            })
        );
        assert_eq!(deserialized_event, event);
        assert_eq!(event.message(), "I compared the two approaches.");
        assert_eq!(event.importance(), ModelEventImportance::Detailed);
        let ModelEvent::Communication(communication) = &event else {
            panic!("the event should be a model communication");
        };
        assert_eq!(communication.subtype(), "reasoning");
        assert_eq!(communication.extensions(), &extensions());
    }

    #[test]
    fn model_communication_rejects_an_empty_message() {
        assert_eq!(
            ModelCommunication::new(
                String::new(),
                ModelEventImportance::Detailed,
                "reasoning".to_owned(),
                Map::new(),
            ),
            Err(InvalidModelCommunication::EmptyMessage)
        );
        assert_eq!(
            ModelCommunication::new(
                " \n ".to_owned(),
                ModelEventImportance::Detailed,
                "reasoning".to_owned(),
                Map::new(),
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
                Map::new(),
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
            Map::new(),
        )
        .expect("the model communication should be valid");

        assert_eq!(communication.message(), "  message\n");
        assert_eq!(communication.subtype(), " reasoning_summary ");
    }

    #[test]
    fn conversation_model_event_round_trip_preserves_source() {
        let source = source();
        let model_event = ModelEvent::AssistantResponse(
            AssistantResponse::new("The answer is 42.".to_owned(), Map::new())
                .expect("the assistant response should be valid"),
        );
        let event = ConversationEvent::from_model_event(source.clone(), model_event.clone());

        let json = serde_json::to_value(&event).expect("the conversation event should serialize");
        let deserialized_event: ConversationEvent = serde_json::from_value(json.clone())
            .expect("the conversation event should deserialize");

        assert_eq!(
            json,
            json!({
                "type": "model",
                "source": { "provider": "openai", "model": "gpt-5.6" },
                "event": {
                    "type": "assistant_response",
                    "message": "The answer is 42.",
                    "extensions": {}
                }
            })
        );
        assert_eq!(deserialized_event, event);
        assert_eq!(
            event,
            ConversationEvent::Model {
                source,
                event: model_event,
            }
        );
    }
}
