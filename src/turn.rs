use std::error::Error;

use crate::conversation::{ConversationEvent, ConversationId, ModelEvent, UserContent, UserPrompt};
use crate::model_driver::ModelDriver;
use crate::persistence::EventStore;

pub(crate) type TurnResultValue<T> = Result<T, Box<dyn Error>>;

pub(crate) struct TurnRequest {
    pub(crate) conversation_id: Option<ConversationId>,
    pub(crate) user_prompt: UserPrompt,
}

pub(crate) struct TurnResult {
    pub(crate) model_events: Vec<ModelEvent>,
}

pub(crate) enum TurnProgress {
    ModelInvocationStarted { model: String },
}

pub(crate) struct TurnService {
    event_store: EventStore,
    model_driver: Box<dyn ModelDriver>,
}

impl TurnService {
    pub(crate) fn new(event_store: EventStore, model_driver: Box<dyn ModelDriver>) -> Self {
        Self {
            event_store,
            model_driver,
        }
    }

    pub(crate) fn execute(
        &self,
        request: TurnRequest,
        conversation_identified: impl FnOnce(ConversationId),
        mut report_progress: impl FnMut(TurnProgress),
    ) -> TurnResultValue<TurnResult> {
        let conversation = match request.conversation_id {
            Some(conversation_id) => self.event_store.load_conversation(conversation_id)?,
            None => self.event_store.create_conversation()?,
        };
        conversation_identified(conversation.id);
        self.event_store.append_conversation_event(
            conversation.id,
            ConversationEvent::User {
                content: vec![UserContent::Text(request.user_prompt.text().to_owned())],
            },
        )?;

        let conversation_events = self
            .event_store
            .load_conversation_events(conversation.id)?
            .into_iter()
            .map(|stored_event| stored_event.event)
            .collect::<Vec<_>>();
        report_progress(TurnProgress::ModelInvocationStarted {
            model: self.model_driver.model().as_str().to_owned(),
        });
        let returned_events = self.model_driver.invoke(&conversation_events)?;
        let mut model_events = Vec::new();
        for event in returned_events {
            if let ConversationEvent::Model { event } = &event {
                model_events.push(event.clone());
            }
            self.event_store
                .append_conversation_event(conversation.id, event)?;
        }

        Ok(TurnResult { model_events })
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use std::sync::{Arc, Mutex};

    use serde_json::{Map, Value};

    use super::{TurnRequest, TurnService};
    use crate::conversation::{
        ConversationEvent, ModelEvent, ModelEventImportance, UserContent, UserPrompt,
    };
    use crate::model_driver::{ModelDriver, ModelDriverError, ModelId};
    use crate::persistence::EventStore;

    struct RecordingModelDriver {
        model: ModelId,
        events: Vec<ConversationEvent>,
        inputs: Arc<Mutex<Vec<Vec<ConversationEvent>>>>,
    }

    impl ModelDriver for RecordingModelDriver {
        fn model(&self) -> &ModelId {
            &self.model
        }

        fn invoke(
            &self,
            conversation: &[ConversationEvent],
        ) -> Result<Vec<ConversationEvent>, ModelDriverError> {
            self.inputs
                .lock()
                .expect("the model input list should lock")
                .push(conversation.to_vec());
            Ok(self.events.clone())
        }
    }

    struct FailingModelDriver {
        model: ModelId,
    }

    impl ModelDriver for FailingModelDriver {
        fn model(&self) -> &ModelId {
            &self.model
        }

        fn invoke(
            &self,
            _conversation: &[ConversationEvent],
        ) -> Result<Vec<ConversationEvent>, ModelDriverError> {
            Err(ModelDriverError::Provider("failure".to_owned()))
        }
    }

    fn model_id() -> ModelId {
        ModelId::from_str("test-model").expect("the model identifier should be valid")
    }

    fn model_event(message: &str, importance: ModelEventImportance) -> ConversationEvent {
        ConversationEvent::Model {
            event: ModelEvent {
                message: message.to_owned(),
                subtype: "test".to_owned(),
                importance,
                data: Map::from_iter([("custom".to_owned(), Value::Bool(true))]),
            },
        }
    }

    fn turn_request(
        conversation_id: crate::conversation::ConversationId,
        prompt: &str,
    ) -> TurnRequest {
        TurnRequest {
            conversation_id: Some(conversation_id),
            user_prompt: UserPrompt::from_str(prompt).expect("the user prompt should be valid"),
        }
    }

    #[test]
    fn another_model_driver_continues_from_semantic_conversation_alone() {
        let root_directory =
            std::env::temp_dir().join(format!("tog-model-driver-test-{}", uuid::Uuid::now_v7()));
        let event_store =
            EventStore::new(root_directory.clone()).expect("the event store should be created");
        let conversation = event_store
            .create_conversation()
            .expect("the conversation should be created");
        let first_service = TurnService::new(
            event_store,
            Box::new(RecordingModelDriver {
                model: model_id(),
                events: vec![model_event("First answer", ModelEventImportance::Important)],
                inputs: Arc::new(Mutex::new(Vec::new())),
            }),
        );
        first_service
            .execute(
                turn_request(conversation.id, "First question"),
                |_| {},
                |_| {},
            )
            .expect("the first turn should complete");

        let second_inputs = Arc::new(Mutex::new(Vec::new()));
        let second_service = TurnService::new(
            EventStore::new(root_directory).expect("the event store should reopen"),
            Box::new(RecordingModelDriver {
                model: model_id(),
                events: vec![model_event(
                    "Second answer",
                    ModelEventImportance::Important,
                )],
                inputs: Arc::clone(&second_inputs),
            }),
        );
        second_service
            .execute(
                turn_request(conversation.id, "Second question"),
                |_| {},
                |_| {},
            )
            .expect("the second turn should complete");

        let recorded_inputs = second_inputs
            .lock()
            .expect("the second model input list should lock");
        assert_eq!(
            recorded_inputs[0],
            [
                ConversationEvent::User {
                    content: vec![UserContent::Text("First question".to_owned())]
                },
                model_event("First answer", ModelEventImportance::Important),
                ConversationEvent::User {
                    content: vec![UserContent::Text("Second question".to_owned())]
                }
            ]
        );
    }

    #[test]
    fn returned_events_are_persisted_with_importance_and_custom_data() {
        let root_directory =
            std::env::temp_dir().join(format!("tog-returned-events-test-{}", uuid::Uuid::now_v7()));
        let event_store =
            EventStore::new(root_directory.clone()).expect("the event store should be created");
        let conversation = event_store
            .create_conversation()
            .expect("the conversation should be created");
        let service = TurnService::new(
            event_store,
            Box::new(RecordingModelDriver {
                model: model_id(),
                events: vec![
                    model_event("Thinking", ModelEventImportance::Detailed),
                    model_event("Answer", ModelEventImportance::Important),
                ],
                inputs: Arc::new(Mutex::new(Vec::new())),
            }),
        );

        let result = service
            .execute(turn_request(conversation.id, "Question"), |_| {}, |_| {})
            .expect("the turn should complete");

        assert_eq!(result.model_events.len(), 2);
        let stored_events = EventStore::new(root_directory)
            .expect("the event store should reopen")
            .load_conversation_events(conversation.id)
            .expect("the events should load");
        assert_eq!(
            stored_events[1].event,
            model_event("Thinking", ModelEventImportance::Detailed)
        );
        let ConversationEvent::Model { event } = &stored_events[1].event else {
            panic!("the event should be a model event");
        };
        assert_eq!(event.data["custom"], true);
    }

    #[test]
    fn failed_invocation_retains_only_the_user_event() {
        let root_directory = std::env::temp_dir().join(format!(
            "tog-model-driver-failure-test-{}",
            uuid::Uuid::now_v7()
        ));
        let event_store =
            EventStore::new(root_directory).expect("the event store should be created");
        let conversation = event_store
            .create_conversation()
            .expect("the conversation should be created");
        let service = TurnService::new(
            event_store,
            Box::new(FailingModelDriver { model: model_id() }),
        );

        let result = service.execute(turn_request(conversation.id, "Question"), |_| {}, |_| {});

        assert!(result.is_err());
        let events = service
            .event_store
            .load_conversation_events(conversation.id)
            .expect("the conversation events should load");
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].event, ConversationEvent::User { .. }));
    }
}
