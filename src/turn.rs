use std::error::Error;

use futures_util::StreamExt;

use crate::conversation::{
    ConversationEventKind, ConversationId, ModelEvent, UserContent, UserPrompt,
};
use crate::model_driver::ModelDriver;
use crate::persistence::EventStore;

pub(crate) type TurnResultValue<T> = Result<T, Box<dyn Error>>;

pub(crate) struct TurnRequest {
    pub(crate) conversation_id: Option<ConversationId>,
    pub(crate) user_prompt: UserPrompt,
}

pub(crate) enum TurnProgress {
    ModelInvocationStarted { model: String },
    ModelEventCompleted { event: ModelEvent },
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

    pub(crate) async fn execute(
        &self,
        request: TurnRequest,
        conversation_identified: impl FnOnce(ConversationId),
        mut report_progress: impl FnMut(TurnProgress) -> TurnResultValue<()>,
    ) -> TurnResultValue<()> {
        let conversation_id = match request.conversation_id {
            Some(conversation_id) => {
                self.event_store.load_conversation(conversation_id)?;
                conversation_id
            }
            None => ConversationId::new(),
        };
        self.event_store.append_conversation_event(
            conversation_id,
            ConversationEventKind::User {
                content: vec![UserContent::Text(request.user_prompt.text().to_owned())],
            },
        )?;
        conversation_identified(conversation_id);

        let conversation = self.event_store.load_conversation(conversation_id)?;
        let source = self.model_driver.source().clone();
        report_progress(TurnProgress::ModelInvocationStarted {
            model: source.model().as_str().to_owned(),
        })?;
        let mut model_events = self.model_driver.invoke(&conversation).await?;
        while let Some(model_event) = model_events.next().await {
            let model_event = model_event?;
            self.event_store.append_conversation_event(
                conversation_id,
                ConversationEventKind::Model {
                    source: source.clone(),
                    event: model_event.clone(),
                },
            )?;
            report_progress(TurnProgress::ModelEventCompleted { event: model_event })?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use std::sync::{Arc, Mutex};

    use futures_util::future::BoxFuture;
    use futures_util::stream;
    use futures_util::{FutureExt, StreamExt};
    use serde_json::{Map, Value};

    use super::{TurnProgress, TurnRequest, TurnService};
    use crate::conversation::{
        AssistantResponse, Conversation, ConversationEventKind, ConversationId, ModelCommunication,
        ModelEvent, ModelEventImportance, ModelId, ModelSource, ProviderId, UserContent,
        UserPrompt,
    };
    use crate::model_driver::{ModelDriver, ModelDriverError, ModelEventStream};
    use crate::persistence::EventStore;

    struct RecordingModelDriver {
        source: ModelSource,
        events: Vec<ModelEvent>,
        inputs: Arc<Mutex<Vec<Conversation>>>,
    }

    impl ModelDriver for RecordingModelDriver {
        fn source(&self) -> &ModelSource {
            &self.source
        }

        fn invoke<'a>(
            &'a self,
            conversation: &'a Conversation,
        ) -> BoxFuture<'a, Result<ModelEventStream, ModelDriverError>> {
            self.inputs
                .lock()
                .expect("the model input list should lock")
                .push(conversation.clone());
            let events = self.events.clone();
            async move { Ok(stream::iter(events.into_iter().map(Ok)).boxed()) }.boxed()
        }
    }

    struct FailingModelDriver {
        source: ModelSource,
    }

    impl ModelDriver for FailingModelDriver {
        fn source(&self) -> &ModelSource {
            &self.source
        }

        fn invoke<'a>(
            &'a self,
            _conversation: &'a Conversation,
        ) -> BoxFuture<'a, Result<ModelEventStream, ModelDriverError>> {
            async { Err(ModelDriverError::Provider("failure".to_owned())) }.boxed()
        }
    }

    struct LateFailingModelDriver {
        source: ModelSource,
        completed_event: ModelEvent,
    }

    impl ModelDriver for LateFailingModelDriver {
        fn source(&self) -> &ModelSource {
            &self.source
        }

        fn invoke<'a>(
            &'a self,
            _conversation: &'a Conversation,
        ) -> BoxFuture<'a, Result<ModelEventStream, ModelDriverError>> {
            let completed_event = self.completed_event.clone();
            async move {
                Ok(stream::iter(vec![
                    Ok(completed_event),
                    Err(ModelDriverError::Provider("late failure".to_owned())),
                ])
                .boxed())
            }
            .boxed()
        }
    }

    fn model_source(provider: &str, model: &str) -> ModelSource {
        ModelSource::new(
            ProviderId::from_str(provider).expect("the provider identifier should be valid"),
            ModelId::from_str(model).expect("the model identifier should be valid"),
        )
    }

    fn assistant_response(message: &str) -> ModelEvent {
        ModelEvent::AssistantResponse(
            AssistantResponse::new(message.to_owned(), Map::new())
                .expect("the assistant response should be valid"),
        )
    }

    fn communication(message: &str, importance: ModelEventImportance) -> ModelEvent {
        ModelEvent::Communication(
            ModelCommunication::new(
                message.to_owned(),
                importance,
                "test".to_owned(),
                Map::from_iter([("custom".to_owned(), Value::Bool(true))]),
            )
            .expect("the model communication should be valid"),
        )
    }

    fn conversation_model_event_kind(
        source: &ModelSource,
        event: ModelEvent,
    ) -> ConversationEventKind {
        ConversationEventKind::Model {
            source: source.clone(),
            event,
        }
    }

    fn turn_request(conversation_id: Option<ConversationId>, prompt: &str) -> TurnRequest {
        TurnRequest {
            conversation_id,
            user_prompt: UserPrompt::from_str(prompt).expect("the user prompt should be valid"),
        }
    }

    #[tokio::test]
    async fn another_model_driver_continues_from_semantic_conversation_alone() {
        let root_directory =
            std::env::temp_dir().join(format!("tog-model-driver-test-{}", uuid::Uuid::now_v7()));
        let event_store =
            EventStore::new(root_directory.clone()).expect("the event store should be created");
        let first_source = model_source("first-provider", "first-model");
        let first_service = TurnService::new(
            event_store,
            Box::new(RecordingModelDriver {
                source: first_source.clone(),
                events: vec![assistant_response("First answer")],
                inputs: Arc::new(Mutex::new(Vec::new())),
            }),
        );
        let mut conversation_id = None;
        first_service
            .execute(
                turn_request(None, "First question"),
                |identified_conversation_id| {
                    conversation_id = Some(identified_conversation_id);
                },
                |_| Ok(()),
            )
            .await
            .expect("the first turn should complete");
        let conversation_id =
            conversation_id.expect("the first turn should identify its conversation");

        let second_inputs = Arc::new(Mutex::new(Vec::new()));
        let second_service = TurnService::new(
            EventStore::new(root_directory).expect("the event store should reopen"),
            Box::new(RecordingModelDriver {
                source: model_source("second-provider", "second-model"),
                events: vec![assistant_response("Second answer")],
                inputs: Arc::clone(&second_inputs),
            }),
        );
        second_service
            .execute(
                turn_request(Some(conversation_id), "Second question"),
                |_| {},
                |_| Ok(()),
            )
            .await
            .expect("the second turn should complete");

        let recorded_inputs = second_inputs
            .lock()
            .expect("the second model input list should lock");
        assert_eq!(recorded_inputs[0].id(), conversation_id);
        let recorded_events = recorded_inputs[0]
            .events()
            .iter()
            .map(|conversation_event| conversation_event.kind.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            recorded_events,
            [
                ConversationEventKind::User {
                    content: vec![UserContent::Text("First question".to_owned())]
                },
                conversation_model_event_kind(&first_source, assistant_response("First answer")),
                ConversationEventKind::User {
                    content: vec![UserContent::Text("Second question".to_owned())]
                }
            ]
        );
    }

    #[tokio::test]
    async fn completed_events_are_persisted_before_they_are_reported() {
        let root_directory =
            std::env::temp_dir().join(format!("tog-returned-events-test-{}", uuid::Uuid::now_v7()));
        let event_store =
            EventStore::new(root_directory.clone()).expect("the event store should be created");
        let source = model_source("test-provider", "test-model");
        let service = TurnService::new(
            event_store,
            Box::new(RecordingModelDriver {
                source: source.clone(),
                events: vec![
                    communication("Thinking", ModelEventImportance::Detailed),
                    assistant_response("Answer"),
                ],
                inputs: Arc::new(Mutex::new(Vec::new())),
            }),
        );

        let identified_conversation_id = Arc::new(Mutex::new(None));
        let identification = Arc::clone(&identified_conversation_id);
        let report_conversation_id = Arc::clone(&identified_conversation_id);
        let reported_event_counts = Arc::new(Mutex::new(Vec::new()));
        let report_counts = Arc::clone(&reported_event_counts);
        let report_root_directory = root_directory.clone();
        service
            .execute(
                turn_request(None, "Question"),
                move |conversation_id| {
                    *identification
                        .lock()
                        .expect("the conversation identification should lock") =
                        Some(conversation_id);
                },
                move |progress| {
                    if let TurnProgress::ModelEventCompleted { .. } = progress {
                        let conversation_id = report_conversation_id
                            .lock()
                            .expect("the conversation identification should lock")
                            .expect("the turn should identify its conversation");
                        let event_count = EventStore::new(report_root_directory.clone())?
                            .load_conversation(conversation_id)?
                            .events()
                            .len();
                        report_counts
                            .lock()
                            .expect("the report count list should lock")
                            .push(event_count);
                    }
                    Ok(())
                },
            )
            .await
            .expect("the turn should complete");
        let conversation_id = identified_conversation_id
            .lock()
            .expect("the conversation identification should lock")
            .expect("the turn should identify its conversation");

        assert_eq!(
            *reported_event_counts
                .lock()
                .expect("the report count list should lock"),
            [2, 3]
        );
        let conversation = EventStore::new(root_directory)
            .expect("the event store should reopen")
            .load_conversation(conversation_id)
            .expect("the conversation should load");
        assert_eq!(
            conversation.events()[1].kind,
            conversation_model_event_kind(
                &source,
                communication("Thinking", ModelEventImportance::Detailed)
            )
        );
        let ConversationEventKind::Model {
            source: event_source,
            event: ModelEvent::AssistantResponse(_),
        } = &conversation.events()[2].kind
        else {
            panic!("the event should be an assistant response");
        };
        assert_eq!(event_source, &source);
    }

    #[tokio::test]
    async fn failed_invocation_retains_only_the_user_event() {
        let root_directory = std::env::temp_dir().join(format!(
            "tog-model-driver-failure-test-{}",
            uuid::Uuid::now_v7()
        ));
        let event_store =
            EventStore::new(root_directory).expect("the event store should be created");
        let service = TurnService::new(
            event_store,
            Box::new(FailingModelDriver {
                source: model_source("test-provider", "test-model"),
            }),
        );

        let mut conversation_id = None;
        let result = service
            .execute(
                turn_request(None, "Question"),
                |identified_conversation_id| {
                    conversation_id = Some(identified_conversation_id);
                },
                |_| Ok(()),
            )
            .await;

        assert!(result.is_err());
        let conversation = service
            .event_store
            .load_conversation(conversation_id.expect("the turn should identify its conversation"))
            .expect("the conversation should load");
        assert_eq!(conversation.events().len(), 1);
        assert!(matches!(
            conversation.events()[0].kind,
            ConversationEventKind::User { .. }
        ));
    }

    #[tokio::test]
    async fn late_stream_failure_preserves_the_completed_model_event() {
        let root_directory = std::env::temp_dir().join(format!(
            "tog-late-model-driver-failure-test-{}",
            uuid::Uuid::now_v7()
        ));
        let service = TurnService::new(
            EventStore::new(root_directory).expect("the event store should be created"),
            Box::new(LateFailingModelDriver {
                source: model_source("test-provider", "test-model"),
                completed_event: assistant_response("Completed answer"),
            }),
        );

        let mut conversation_id = None;
        let result = service
            .execute(
                turn_request(None, "Question"),
                |identified_conversation_id| conversation_id = Some(identified_conversation_id),
                |_| Ok(()),
            )
            .await;

        assert!(result.is_err());
        let conversation = service
            .event_store
            .load_conversation(conversation_id.expect("the turn should identify its conversation"))
            .expect("the conversation should load");
        assert_eq!(conversation.events().len(), 2);
        assert_eq!(
            conversation.events()[1].kind,
            conversation_model_event_kind(
                service.model_driver.source(),
                assistant_response("Completed answer")
            )
        );
    }

    #[tokio::test]
    async fn nonexistent_conversation_is_not_created_by_a_turn() {
        let root_directory = std::env::temp_dir().join(format!(
            "tog-missing-conversation-test-{}",
            uuid::Uuid::now_v7()
        ));
        let event_store =
            EventStore::new(root_directory.clone()).expect("the event store should be created");
        let conversation_id = ConversationId::new();
        let inputs = Arc::new(Mutex::new(Vec::new()));
        let service = TurnService::new(
            event_store,
            Box::new(RecordingModelDriver {
                source: model_source("test-provider", "test-model"),
                events: Vec::new(),
                inputs: Arc::clone(&inputs),
            }),
        );

        let result = service
            .execute(
                turn_request(Some(conversation_id), "Question"),
                |_| {},
                |_| Ok(()),
            )
            .await;

        assert!(result.is_err());
        assert!(
            inputs
                .lock()
                .expect("the model input list should lock")
                .is_empty()
        );
        assert!(
            !root_directory
                .join("conversations")
                .join(conversation_id.storage_key())
                .exists()
        );
    }
}
