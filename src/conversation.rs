mod event;
mod id;
mod model;

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::str::FromStr;

pub(crate) use event::{
    AssistantResponse, ConversationEvent, InvalidAssistantResponse, InvalidModelCommunication,
    ModelCommunication, ModelEvent, ModelEventImportance, StoredConversationEvent, UserContent,
};
pub(crate) use id::{ConversationEventId, ConversationId};
pub(crate) use model::{ModelId, ModelSource, ProviderId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Conversation {
    id: ConversationId,
    events: Vec<StoredConversationEvent>,
}

impl Conversation {
    pub(crate) fn from_events(
        events: Vec<StoredConversationEvent>,
    ) -> Result<Self, InvalidConversation> {
        let conversation_id = events
            .first()
            .map(|event| event.conversation_id)
            .ok_or(InvalidConversation::Empty)?;

        for (expected_position, event) in events.iter().enumerate() {
            if event.conversation_id != conversation_id {
                return Err(InvalidConversation::MixedConversationIds {
                    expected: conversation_id,
                    found: event.conversation_id,
                });
            }

            let expected_position =
                u64::try_from(expected_position).map_err(|_| InvalidConversation::TooManyEvents)?;
            if event.position != expected_position {
                return Err(InvalidConversation::InvalidPosition {
                    expected: expected_position,
                    found: event.position,
                });
            }

            event
                .event
                .ensure_valid()
                .map_err(|error| InvalidConversation::InvalidEvent {
                    position: event.position,
                    reason: error.to_string(),
                })?;
        }

        Ok(Self {
            id: conversation_id,
            events,
        })
    }

    pub(crate) fn id(&self) -> ConversationId {
        self.id
    }

    pub(crate) fn events(&self) -> &[StoredConversationEvent] {
        &self.events
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum InvalidConversation {
    Empty,
    MixedConversationIds {
        expected: ConversationId,
        found: ConversationId,
    },
    InvalidPosition {
        expected: u64,
        found: u64,
    },
    InvalidEvent {
        position: u64,
        reason: String,
    },
    TooManyEvents,
}

impl Display for InvalidConversation {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(formatter, "a conversation must contain at least one event"),
            Self::MixedConversationIds { expected, found } => write!(
                formatter,
                "conversation event belongs to {found}, expected {expected}"
            ),
            Self::InvalidPosition { expected, found } => {
                write!(
                    formatter,
                    "expected conversation event position {expected}, found {found}"
                )
            }
            Self::InvalidEvent { position, reason } => {
                write!(
                    formatter,
                    "invalid conversation event at position {position}: {reason}"
                )
            }
            Self::TooManyEvents => write!(formatter, "conversation contains too many events"),
        }
    }
}

impl Error for InvalidConversation {}

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

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use serde_json::json;

    use super::{
        Conversation, ConversationEvent, ConversationEventId, ConversationId, InvalidConversation,
        InvalidUserPrompt, StoredConversationEvent, UserContent, UserPrompt,
    };

    fn stored_user_event(
        conversation_id: ConversationId,
        position: u64,
    ) -> StoredConversationEvent {
        StoredConversationEvent {
            conversation_id,
            position,
            id: ConversationEventId::new(),
            timestamp_milliseconds: position,
            schema_version: 5,
            event: ConversationEvent::User {
                content: vec![UserContent::Text(format!("event {position}"))],
            },
        }
    }

    #[test]
    fn conversation_requires_an_event() {
        assert_eq!(
            Conversation::from_events(Vec::new()),
            Err(InvalidConversation::Empty)
        );
    }

    #[test]
    fn conversation_rejects_mixed_conversation_ids() {
        let first_conversation_id = ConversationId::new();
        let second_conversation_id = ConversationId::new();

        let result = Conversation::from_events(vec![
            stored_user_event(first_conversation_id, 0),
            stored_user_event(second_conversation_id, 1),
        ]);

        assert_eq!(
            result,
            Err(InvalidConversation::MixedConversationIds {
                expected: first_conversation_id,
                found: second_conversation_id,
            })
        );
    }

    #[test]
    fn conversation_rejects_invalid_event_order() {
        let conversation_id = ConversationId::new();

        let result = Conversation::from_events(vec![stored_user_event(conversation_id, 1)]);

        assert_eq!(
            result,
            Err(InvalidConversation::InvalidPosition {
                expected: 0,
                found: 1,
            })
        );
    }

    #[test]
    fn conversation_exposes_its_id_and_ordered_events() {
        let conversation_id = ConversationId::new();

        let conversation = Conversation::from_events(vec![
            stored_user_event(conversation_id, 0),
            stored_user_event(conversation_id, 1),
        ])
        .expect("the conversation should be valid");

        assert_eq!(conversation.id(), conversation_id);
        assert_eq!(conversation.events().len(), 2);
        assert_eq!(conversation.events()[0].position, 0);
        assert_eq!(conversation.events()[1].position, 1);
    }

    #[test]
    fn conversation_rejects_invalid_deserialized_model_events() {
        let conversation_id = ConversationId::new();
        let event_id = ConversationEventId::new();
        let stored_event: StoredConversationEvent = serde_json::from_value(json!({
            "conversation_id": conversation_id,
            "position": 0,
            "id": event_id,
            "timestamp_milliseconds": 1,
            "schema_version": 5,
            "event": {
                "type": "model",
                "source": {
                    "provider": "openai",
                    "model": "gpt-5.6"
                },
                "event": {
                    "type": "assistant_response",
                    "message": "   ",
                    "extensions": {}
                }
            }
        }))
        .expect("derived deserialization should construct the stored event");

        assert_eq!(
            Conversation::from_events(vec![stored_event]),
            Err(InvalidConversation::InvalidEvent {
                position: 0,
                reason: "assistant response message must not be empty".to_owned(),
            })
        );
    }

    #[test]
    fn conversation_rejects_invalid_deserialized_model_communications() {
        let conversation_id = ConversationId::new();
        let event_id = ConversationEventId::new();
        let stored_event: StoredConversationEvent = serde_json::from_value(json!({
            "conversation_id": conversation_id,
            "position": 0,
            "id": event_id,
            "timestamp_milliseconds": 1,
            "schema_version": 5,
            "event": {
                "type": "model",
                "source": {
                    "provider": "openai",
                    "model": "gpt-5.6"
                },
                "event": {
                    "type": "communication",
                    "message": "reasoning",
                    "importance": "detailed",
                    "subtype": "   ",
                    "extensions": {}
                }
            }
        }))
        .expect("derived deserialization should construct the stored event");

        assert_eq!(
            Conversation::from_events(vec![stored_event]),
            Err(InvalidConversation::InvalidEvent {
                position: 0,
                reason: "model communication subtype must not be empty".to_owned(),
            })
        );
    }

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
