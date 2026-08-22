use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

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
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct AssistantResponse {
    message: String,
    extensions: Map<String, Value>,
}

impl AssistantResponse {
    pub(crate) fn new(message: String, extensions: Map<String, Value>) -> Self {
        Self {
            message,
            extensions,
        }
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }
}

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
    ) -> Self {
        Self {
            message,
            importance,
            subtype,
            extensions,
        }
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn importance(&self) -> ModelEventImportance {
        self.importance
    }
}

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
    pub(crate) timestamp_milliseconds: u64,
    pub(crate) schema_version: u32,
    pub(crate) event: ConversationEvent,
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use serde_json::{Map, Value, json};

    use super::{
        AssistantResponse, ConversationEvent, ModelCommunication, ModelEvent, ModelEventImportance,
    };
    use crate::conversation::{ModelId, ModelSource, ProviderId};

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

    #[test]
    fn assistant_response_round_trips_through_json() {
        let event = ModelEvent::AssistantResponse(AssistantResponse::new(
            "The answer is 42.".to_owned(),
            extensions(),
        ));

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
    }

    #[test]
    fn model_communication_round_trips_through_json() {
        let event = ModelEvent::Communication(ModelCommunication::new(
            "I compared the two approaches.".to_owned(),
            ModelEventImportance::Detailed,
            "reasoning".to_owned(),
            extensions(),
        ));

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
    }

    #[test]
    fn conversation_model_event_round_trip_preserves_source() {
        let event = ConversationEvent::Model {
            source: source(),
            event: ModelEvent::AssistantResponse(AssistantResponse::new(
                "The answer is 42.".to_owned(),
                Map::new(),
            )),
        };

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
    }
}
