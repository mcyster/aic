use std::error::Error;

use futures_util::StreamExt;

use crate::conversation::{
    ConversationEventKind, ConversationId, ConversationProblem, InvalidConversationProblem,
    InvocationError, ModelEvent, UserContent, UserPrompt,
};
use crate::model_driver::{ModelDriver, ModelDriverError, ModelDriverEvent};
use crate::persistence::EventStore;

pub(crate) type TurnResultValue<T> = Result<T, Box<dyn Error>>;

pub(crate) struct TurnRequest {
    pub(crate) conversation_id: Option<ConversationId>,
    pub(crate) user_prompt: UserPrompt,
}

pub(crate) enum TurnProgress {
    InvocationStarted { model: String },
    EventCompleted { event: ModelEvent },
    ProblemCompleted { problem: ConversationProblem },
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
            None,
        )?;
        conversation_identified(conversation_id);

        let conversation = self.event_store.load_conversation(conversation_id)?;
        let source = self.model_driver.source().clone();
        report_progress(TurnProgress::InvocationStarted {
            model: source.model().as_str().to_owned(),
        })?;
        let mut driver_events = match self.model_driver.invoke(&conversation).await {
            Ok(driver_events) => driver_events,
            Err(error) => {
                self.append_invocation_problem(
                    conversation_id,
                    &error,
                    InvocationStage::BeforeStream,
                )?;
                return Err(Box::new(error));
            }
        };
        while let Some(driver_event) = driver_events.next().await {
            let driver_event = match driver_event {
                Ok(driver_event) => driver_event,
                Err(error) => {
                    self.append_invocation_problem(
                        conversation_id,
                        &error,
                        InvocationStage::DuringStream,
                    )?;
                    return Err(Box::new(error));
                }
            };
            match driver_event {
                ModelDriverEvent::Model { event, data } => {
                    self.event_store.append_conversation_event(
                        conversation_id,
                        ConversationEventKind::Model {
                            source: source.clone(),
                            event: event.clone(),
                        },
                        data,
                    )?;
                    report_progress(TurnProgress::EventCompleted { event })?;
                }
                ModelDriverEvent::Problem { problem, data } => {
                    let conversation_problem = ConversationProblem::Issue(problem);
                    self.event_store.append_conversation_event(
                        conversation_id,
                        ConversationEventKind::Problem {
                            problem: conversation_problem.clone(),
                        },
                        data,
                    )?;
                    report_progress(TurnProgress::ProblemCompleted {
                        problem: conversation_problem,
                    })?;
                }
            }
        }

        Ok(())
    }

    fn append_invocation_problem(
        &self,
        conversation_id: ConversationId,
        error: &ModelDriverError,
        stage: InvocationStage,
    ) -> TurnResultValue<()> {
        let problem = invocation_problem(error, stage)?;
        self.event_store.append_conversation_event(
            conversation_id,
            ConversationEventKind::Problem { problem },
            None,
        )?;
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum InvocationStage {
    BeforeStream,
    DuringStream,
}

fn invocation_problem(
    error: &ModelDriverError,
    stage: InvocationStage,
) -> Result<ConversationProblem, InvalidConversationProblem> {
    let invocation_error = match error {
        ModelDriverError::Authentication(_) => InvocationError::try_authentication(
            "The model provider could not authenticate the invocation.".to_owned(),
        )?,
        ModelDriverError::RateLimited(_) => InvocationError::try_rate_limited(
            "The model provider rate-limited the invocation.".to_owned(),
        )?,
        ModelDriverError::Transport(_) if matches!(stage, InvocationStage::DuringStream) => {
            InvocationError::try_stream_interrupted(
                "The model response stream was interrupted.".to_owned(),
            )?
        }
        ModelDriverError::Transport(_) => {
            InvocationError::try_transport("The model provider could not be reached.".to_owned())?
        }
        ModelDriverError::InvalidRequest(_) => InvocationError::try_invalid_request(
            "The model invocation request was invalid.".to_owned(),
        )?,
        ModelDriverError::InvalidResponse(_) => InvocationError::try_invalid_provider_response(
            "The model provider returned an invalid response.".to_owned(),
        )?,
        ModelDriverError::StreamInterrupted(_) => InvocationError::try_stream_interrupted(
            "The model response stream was interrupted.".to_owned(),
        )?,
        ModelDriverError::Provider(_) => InvocationError::try_provider_failure(
            "The model provider failed the invocation.".to_owned(),
        )?,
    };
    Ok(ConversationProblem::Invocation(invocation_error))
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
        AssistantResponse, Conversation, ConversationEventKind, ConversationId,
        ConversationProblem, InvocationError, ModelCommunication, ModelData, ModelEvent,
        ModelEventImportance, ModelId, ModelIssue, ModelSource, ProviderId, UserContent,
        UserPrompt,
    };
    use crate::model_driver::{ModelDriver, ModelDriverError, ModelDriverEvent, ModelOutputStream};
    use crate::persistence::EventStore;

    struct RecordingModelDriver {
        source: ModelSource,
        events: Vec<ModelEvent>,
        data: Option<ModelData>,
        inputs: Arc<Mutex<Vec<Conversation>>>,
    }

    impl ModelDriver for RecordingModelDriver {
        fn source(&self) -> &ModelSource {
            &self.source
        }

        fn invoke<'invoke>(
            &'invoke self,
            conversation: &'invoke Conversation,
        ) -> BoxFuture<'invoke, Result<ModelOutputStream, ModelDriverError>> {
            self.inputs
                .lock()
                .expect("the model input list should lock")
                .push(conversation.clone());
            let events = self.events.clone();
            let data = self.data.clone();
            async move {
                Ok(stream::iter(
                    events
                        .into_iter()
                        .map(move |event| ModelDriverEvent::Model {
                            event,
                            data: data.clone(),
                        })
                        .map(Ok),
                )
                .boxed())
            }
            .boxed()
        }
    }

    struct FailingModelDriver {
        source: ModelSource,
    }

    impl ModelDriver for FailingModelDriver {
        fn source(&self) -> &ModelSource {
            &self.source
        }

        fn invoke<'invoke>(
            &'invoke self,
            _conversation: &'invoke Conversation,
        ) -> BoxFuture<'invoke, Result<ModelOutputStream, ModelDriverError>> {
            async { Err(ModelDriverError::Provider("failure".to_owned())) }.boxed()
        }
    }

    struct LateFailingModelDriver {
        source: ModelSource,
        completed_event: ModelEvent,
    }

    struct IssueModelDriver {
        source: ModelSource,
        issue: ModelIssue,
        data: Option<ModelData>,
    }

    impl ModelDriver for IssueModelDriver {
        fn source(&self) -> &ModelSource {
            &self.source
        }

        fn invoke<'invoke>(
            &'invoke self,
            _conversation: &'invoke Conversation,
        ) -> BoxFuture<'invoke, Result<ModelOutputStream, ModelDriverError>> {
            let issue = self.issue.clone();
            let data = self.data.clone();
            async move {
                Ok(stream::once(async move {
                    Ok(ModelDriverEvent::Problem {
                        problem: issue,
                        data,
                    })
                })
                .boxed())
            }
            .boxed()
        }
    }

    impl ModelDriver for LateFailingModelDriver {
        fn source(&self) -> &ModelSource {
            &self.source
        }

        fn invoke<'invoke>(
            &'invoke self,
            _conversation: &'invoke Conversation,
        ) -> BoxFuture<'invoke, Result<ModelOutputStream, ModelDriverError>> {
            let completed_event = self.completed_event.clone();
            async move {
                Ok(stream::iter(vec![
                    Ok(ModelDriverEvent::Model {
                        event: completed_event,
                        data: None,
                    }),
                    Err(ModelDriverError::Transport("late failure".to_owned())),
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

    fn model_data(provider: &str) -> ModelData {
        ModelData::new(
            ProviderId::from_str(provider).expect("the provider identifier should be valid"),
            Map::from_iter([("response_id".to_owned(), Value::String("resp_1".to_owned()))]),
        )
        .expect("the model data should be valid")
    }

    fn assistant_response(message: &str) -> ModelEvent {
        ModelEvent::AssistantResponse(
            AssistantResponse::new(message.to_owned())
                .expect("the assistant response should be valid"),
        )
    }

    fn communication(message: &str, importance: ModelEventImportance) -> ModelEvent {
        ModelEvent::Communication(
            ModelCommunication::new(message.to_owned(), importance, "test".to_owned())
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
                data: None,
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
                data: None,
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
                data: None,
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
                    if let TurnProgress::EventCompleted { .. } = progress {
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
    async fn failed_invocation_persists_a_sanitized_problem() {
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
        assert_eq!(conversation.events().len(), 2);
        assert!(matches!(
            conversation.events()[0].kind,
            ConversationEventKind::User { .. }
        ));
        let ConversationEventKind::Problem { problem } = &conversation.events()[1].kind else {
            panic!("the second event should be an invocation problem");
        };
        assert_eq!(
            problem.message(),
            "The model provider failed the invocation."
        );
        assert!(matches!(
            problem,
            ConversationProblem::Invocation(InvocationError::ProviderFailure { .. })
        ));
    }

    #[tokio::test]
    async fn model_issue_is_persisted_and_reported_as_a_top_level_problem() {
        let root_directory =
            std::env::temp_dir().join(format!("tog-model-issue-test-{}", uuid::Uuid::now_v7()));
        let source = model_source("test-provider", "test-model");
        let service = TurnService::new(
            EventStore::new(root_directory).expect("the event store should be created"),
            Box::new(IssueModelDriver {
                source,
                issue: ModelIssue::try_refusal("Refused.".to_owned())
                    .expect("the refusal should be valid"),
                data: None,
            }),
        );
        let reported_problems = Arc::new(Mutex::new(Vec::new()));
        let problem_reports = Arc::clone(&reported_problems);
        let mut conversation_id = None;

        service
            .execute(
                turn_request(None, "Question"),
                |identified_conversation_id| conversation_id = Some(identified_conversation_id),
                move |progress| {
                    if let TurnProgress::ProblemCompleted { problem } = progress {
                        problem_reports
                            .lock()
                            .expect("the problem report list should lock")
                            .push(problem);
                    }
                    Ok(())
                },
            )
            .await
            .expect("the model issue turn should complete");

        let conversation = service
            .event_store
            .load_conversation(conversation_id.expect("the turn should identify its conversation"))
            .expect("the conversation should load");
        assert!(matches!(
            &conversation.events()[1].kind,
            ConversationEventKind::Problem {
                problem: ConversationProblem::Issue(ModelIssue::Refusal { .. }),
            }
        ));
        let reports = reported_problems
            .lock()
            .expect("the problem report list should lock");
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].message(), "Refused.");
    }

    #[tokio::test]
    async fn model_data_supplied_with_a_model_event_is_persisted_on_the_envelope() {
        let root_directory =
            std::env::temp_dir().join(format!("tog-model-data-test-{}", uuid::Uuid::now_v7()));
        let service = TurnService::new(
            EventStore::new(root_directory).expect("the event store should be created"),
            Box::new(RecordingModelDriver {
                source: model_source("test-provider", "test-model"),
                events: vec![assistant_response("Answer")],
                data: Some(model_data("test-provider")),
                inputs: Arc::new(Mutex::new(Vec::new())),
            }),
        );
        let mut conversation_id = None;

        service
            .execute(
                turn_request(None, "Question"),
                |identified_conversation_id| conversation_id = Some(identified_conversation_id),
                |_| Ok(()),
            )
            .await
            .expect("the turn should complete");

        let conversation = service
            .event_store
            .load_conversation(conversation_id.expect("the turn should identify its conversation"))
            .expect("the conversation should load");
        assert!(matches!(
            conversation.events()[1].kind,
            ConversationEventKind::Model { .. }
        ));
        assert_eq!(conversation.events()[0].model, None);
        assert_eq!(
            conversation.events()[1].model,
            Some(model_data("test-provider"))
        );
    }

    #[tokio::test]
    async fn model_data_supplied_with_a_model_problem_is_persisted_on_the_envelope() {
        let root_directory = std::env::temp_dir().join(format!(
            "tog-problem-model-data-test-{}",
            uuid::Uuid::now_v7()
        ));
        let service = TurnService::new(
            EventStore::new(root_directory).expect("the event store should be created"),
            Box::new(IssueModelDriver {
                source: model_source("test-provider", "test-model"),
                issue: ModelIssue::try_refusal("Refused.".to_owned())
                    .expect("the refusal should be valid"),
                data: Some(model_data("test-provider")),
            }),
        );
        let mut conversation_id = None;

        service
            .execute(
                turn_request(None, "Question"),
                |identified_conversation_id| conversation_id = Some(identified_conversation_id),
                |_| Ok(()),
            )
            .await
            .expect("the model issue turn should complete");

        let conversation = service
            .event_store
            .load_conversation(conversation_id.expect("the turn should identify its conversation"))
            .expect("the conversation should load");
        assert!(matches!(
            conversation.events()[1].kind,
            ConversationEventKind::Problem { .. }
        ));
        assert_eq!(
            conversation.events()[1].model,
            Some(model_data("test-provider"))
        );
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
        assert_eq!(conversation.events().len(), 3);
        assert_eq!(
            conversation.events()[1].kind,
            conversation_model_event_kind(
                service.model_driver.source(),
                assistant_response("Completed answer")
            )
        );
        assert!(matches!(
            conversation.events()[2].kind,
            ConversationEventKind::Problem {
                problem: ConversationProblem::Invocation(InvocationError::StreamInterrupted { .. }),
            }
        ));
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
                data: None,
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
