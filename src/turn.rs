use std::error::Error;

use futures_util::StreamExt;

use crate::conversation::{
    ConversationCommandId, ConversationEvent, ConversationEventKind, ConversationId,
    ConversationProblem, ConversationTurnId, UserContent, UserPrompt,
};
use crate::model_driver::{ModelDriver, ModelDriverError};
use crate::persistence::EventStore;

pub(crate) type TurnResultValue<T> = Result<T, Box<dyn Error>>;

pub(crate) struct TurnRequest {
    pub(crate) conversation_id: Option<ConversationId>,
    pub(crate) user_prompt: UserPrompt,
}

pub(crate) enum TurnProgress {
    InvocationStarted { model: String },
    EventCompleted { event: ConversationEventKind },
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
        let user_content = vec![UserContent::Text(request.user_prompt.text().to_owned())];
        let user_message_command_id = ConversationCommandId::new();
        self.event_store.append_new_conversation_event(
            conversation_id,
            ConversationEvent::Shared(ConversationEventKind::UserMessageRequested {
                command_id: user_message_command_id,
                content: user_content.clone(),
            }),
        )?;
        self.event_store.append_new_conversation_event(
            conversation_id,
            ConversationEvent::Shared(ConversationEventKind::User {
                caused_by: Some(user_message_command_id),
                content: user_content,
            }),
        )?;
        conversation_identified(conversation_id);

        let source = self.model_driver.source().clone();
        let turn_id = ConversationTurnId::new();
        self.event_store.append_new_conversation_event(
            conversation_id,
            ConversationEvent::Shared(ConversationEventKind::TurnRequested {
                command_id: ConversationCommandId::new(),
                turn_id,
                model: source.clone(),
            }),
        )?;
        let conversation = self.event_store.load_conversation(conversation_id)?;
        report_progress(TurnProgress::InvocationStarted {
            model: source.model().as_str().to_owned(),
        })?;

        let mut conversation_events = match self.model_driver.invoke(&conversation, turn_id).await {
            Ok(conversation_events) => conversation_events,
            Err(error) => return Err(Box::new(error)),
        };

        let mut turn_completed = false;
        while let Some(conversation_event) = conversation_events.next().await {
            let driver_output = match conversation_event {
                Ok(driver_output) => driver_output,
                Err(error) => return Err(Box::new(error)),
            };
            if turn_completed {
                return Err(Box::new(ModelDriverError::InvalidOutput(
                    "the model driver emitted output after completing the turn".to_owned(),
                )));
            }
            match driver_output {
                ConversationEvent::Extension(event) => {
                    self.event_store.append_new_conversation_event(
                        conversation_id,
                        ConversationEvent::Extension(event),
                    )?;
                }
                ConversationEvent::Shared(conversation_kind) => match &conversation_kind {
                    ConversationEventKind::Assistant { .. }
                    | ConversationEventKind::Communication { .. } => {
                        ensure_event_belongs_to_turn(&conversation_kind, turn_id)?;
                        self.event_store.append_new_conversation_event(
                            conversation_id,
                            ConversationEvent::Shared(conversation_kind.clone()),
                        )?;
                        report_progress(TurnProgress::EventCompleted {
                            event: conversation_kind,
                        })?;
                    }
                    ConversationEventKind::Problem { problem, .. } => {
                        ensure_event_belongs_to_turn(&conversation_kind, turn_id)?;
                        let problem = problem.clone();
                        self.event_store.append_new_conversation_event(
                            conversation_id,
                            ConversationEvent::Shared(conversation_kind),
                        )?;
                        report_progress(TurnProgress::ProblemCompleted { problem })?;
                    }
                    ConversationEventKind::TurnCompleted { .. } => {
                        ensure_event_belongs_to_turn(&conversation_kind, turn_id)?;
                        if turn_completed {
                            let error = ModelDriverError::InvalidOutput(
                                "the model driver completed the turn more than once".to_owned(),
                            );
                            return Err(Box::new(error));
                        }
                        turn_completed = true;
                        self.event_store.append_new_conversation_event(
                            conversation_id,
                            ConversationEvent::Shared(conversation_kind),
                        )?;
                    }
                    _ => {
                        let error = ModelDriverError::InvalidOutput(
                            "the model driver returned an invalid conversation event".to_owned(),
                        );
                        return Err(Box::new(error));
                    }
                },
            }
        }

        if !turn_completed {
            return Err(Box::new(ModelDriverError::IncompleteTurn));
        }
        Ok(())
    }
}

fn ensure_event_belongs_to_turn(
    event: &ConversationEventKind,
    expected_turn_id: ConversationTurnId,
) -> Result<(), ModelDriverError> {
    let actual_turn_id = match event {
        ConversationEventKind::Assistant { turn_id, .. }
        | ConversationEventKind::Communication { turn_id, .. }
        | ConversationEventKind::TurnCompleted { turn_id, .. } => *turn_id,
        ConversationEventKind::Problem {
            turn_id: Some(turn_id),
            ..
        } => *turn_id,
        ConversationEventKind::Problem { turn_id: None, .. } => {
            return Err(ModelDriverError::InvalidOutput(
                "the model driver returned a problem without a turn".to_owned(),
            ));
        }
        _ => {
            return Err(ModelDriverError::InvalidOutput(
                "the model driver returned an invalid conversation event".to_owned(),
            ));
        }
    };
    if actual_turn_id != expected_turn_id {
        return Err(ModelDriverError::InvalidOutput(
            "the model driver returned an event for another turn".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use std::sync::{Arc, Mutex};

    use futures_util::future::BoxFuture;
    use futures_util::stream;
    use futures_util::{FutureExt, StreamExt};

    use super::{TurnRequest, TurnService};
    use crate::conversation::DriverEventReadError;
    use crate::conversation::DriverEventReader;
    use crate::conversation::{
        AssistantResponse, Conversation, ConversationEvent, ConversationEventError,
        ConversationEventExtension, ConversationEventKind, ConversationEventRecord, ConversationId,
        ConversationProblem, ConversationTurnId, DriverEventEnvelope, ModelCommunication,
        ModelDetails, ModelEventImportance, ModelId, ModelInvocationId, ModelIssue, ModelSource,
        ProviderId, StoredConversationEventKind, TurnOutcome, UserPrompt,
    };
    use crate::model_driver::{ModelDriver, ModelDriverError, ModelOutputStream};
    use crate::persistence::EventStore;

    enum TestOutput {
        Assistant(String),
        Communication(String),
        Problem(ConversationProblem),
    }

    struct RecordingModelDriver {
        source: ModelSource,
        outputs: Vec<TestOutput>,
        inputs: Arc<Mutex<Vec<Conversation>>>,
        invocations: Arc<Mutex<Vec<(ConversationTurnId, ModelInvocationId)>>>,
    }

    struct TestInvocationRequested {
        invocation_id: ModelInvocationId,
        turn_id: ConversationTurnId,
    }

    impl ConversationEventExtension for TestInvocationRequested {
        fn driver_name(&self) -> &str {
            "test"
        }

        fn driver_version(&self) -> &str {
            "1"
        }

        fn event_type(&self) -> &str {
            "model_invocation_requested"
        }

        fn event_schema_version(&self) -> u32 {
            1
        }

        fn description(&self) -> &str {
            "Test model invocation was requested."
        }

        fn serialize_payload(&self) -> Result<serde_json::Value, ConversationEventError> {
            Ok(serde_json::json!({
                "invocation_id": self.invocation_id,
                "turn_id": self.turn_id,
            }))
        }
    }

    #[allow(dead_code)]
    fn unsupported_event(
        _event: &DriverEventEnvelope,
    ) -> Result<Box<dyn ConversationEventExtension>, DriverEventReadError> {
        Err(DriverEventReadError::UnsupportedDriver)
    }

    impl ModelDriver for RecordingModelDriver {
        fn source(&self) -> &ModelSource {
            &self.source
        }

        fn invoke<'invoke>(
            &'invoke self,
            conversation: &'invoke Conversation,
            turn_id: ConversationTurnId,
        ) -> BoxFuture<'invoke, Result<ModelOutputStream, ModelDriverError>> {
            self.inputs
                .lock()
                .expect("the model input list should lock")
                .push(conversation.clone());
            let invocation_id = ModelInvocationId::new();
            self.invocations
                .lock()
                .expect("the invocation list should lock")
                .push((turn_id, invocation_id));
            let mut outputs = vec![Ok(ConversationEvent::Extension(Box::new(
                TestInvocationRequested {
                    invocation_id,
                    turn_id,
                },
            )
                as Box<dyn ConversationEventExtension>))];
            let semantic_outputs = self
                .outputs
                .iter()
                .map(|output| output_kind(output, &self.source, turn_id, invocation_id))
                .collect::<Vec<_>>();
            let turn_outcome = if self
                .outputs
                .iter()
                .any(|output| matches!(output, TestOutput::Problem(_)))
            {
                TurnOutcome::Failed
            } else {
                TurnOutcome::Succeeded
            };
            outputs.extend(
                semantic_outputs
                    .into_iter()
                    .map(|output| Ok(ConversationEvent::Shared(output))),
            );
            outputs.push(Ok(ConversationEvent::Shared(
                ConversationEventKind::TurnCompleted {
                    turn_id,
                    outcome: turn_outcome,
                },
            )));
            async move { Ok(stream::iter(outputs).boxed()) }.boxed()
        }
    }

    impl DriverEventReader for RecordingModelDriver {
        fn read_event(
            &self,
            event: &DriverEventEnvelope,
        ) -> Result<Box<dyn ConversationEventExtension>, DriverEventReadError> {
            unsupported_event(event)
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
            _turn_id: ConversationTurnId,
        ) -> BoxFuture<'invoke, Result<ModelOutputStream, ModelDriverError>> {
            async { Err(ModelDriverError::InvalidOutput("failure".to_owned())) }.boxed()
        }
    }

    impl DriverEventReader for FailingModelDriver {
        fn read_event(
            &self,
            event: &DriverEventEnvelope,
        ) -> Result<Box<dyn ConversationEventExtension>, DriverEventReadError> {
            unsupported_event(event)
        }
    }

    struct LateFailingModelDriver {
        source: ModelSource,
    }

    impl ModelDriver for LateFailingModelDriver {
        fn source(&self) -> &ModelSource {
            &self.source
        }

        fn invoke<'invoke>(
            &'invoke self,
            _conversation: &'invoke Conversation,
            turn_id: ConversationTurnId,
        ) -> BoxFuture<'invoke, Result<ModelOutputStream, ModelDriverError>> {
            let source = self.source.clone();
            let invocation_id = ModelInvocationId::new();
            async move {
                Ok(stream::iter(vec![
                    Ok(ConversationEvent::Shared(assistant_kind(
                        "Completed answer",
                        &source,
                        turn_id,
                        invocation_id,
                    ))),
                    Err(ModelDriverError::InvalidOutput("late failure".to_owned())),
                ])
                .boxed())
            }
            .boxed()
        }
    }

    impl DriverEventReader for LateFailingModelDriver {
        fn read_event(
            &self,
            event: &DriverEventEnvelope,
        ) -> Result<Box<dyn ConversationEventExtension>, DriverEventReadError> {
            unsupported_event(event)
        }
    }

    fn output_kind(
        output: &TestOutput,
        source: &ModelSource,
        turn_id: ConversationTurnId,
        invocation_id: ModelInvocationId,
    ) -> ConversationEventKind {
        match output {
            TestOutput::Assistant(message) => {
                assistant_kind(message, source, turn_id, invocation_id)
            }
            TestOutput::Communication(message) => ConversationEventKind::Communication {
                turn_id,
                invocation_id,
                model: ModelDetails::new(source.clone(), None)
                    .expect("the model details should be valid"),
                communication: ModelCommunication::new(
                    message.clone(),
                    ModelEventImportance::Detailed,
                    "test".to_owned(),
                )
                .expect("the communication should be valid"),
            },
            TestOutput::Problem(problem) => ConversationEventKind::Problem {
                turn_id: Some(turn_id),
                invocation_id: Some(invocation_id),
                model: Some(
                    ModelDetails::new(source.clone(), None)
                        .expect("the model details should be valid"),
                ),
                problem: problem.clone(),
            },
        }
    }

    fn assistant_kind(
        message: &str,
        source: &ModelSource,
        turn_id: ConversationTurnId,
        invocation_id: ModelInvocationId,
    ) -> ConversationEventKind {
        ConversationEventKind::Assistant {
            turn_id,
            invocation_id,
            model: ModelDetails::new(source.clone(), None)
                .expect("the model details should be valid"),
            response: AssistantResponse::new(message.to_owned())
                .expect("the assistant response should be valid"),
        }
    }

    fn model_source(provider: &str, model: &str) -> ModelSource {
        ModelSource::new(
            ProviderId::from_str(provider).expect("the provider identifier should be valid"),
            ModelId::from_str(model).expect("the model identifier should be valid"),
        )
    }

    fn turn_request(conversation_id: Option<ConversationId>, prompt: &str) -> TurnRequest {
        TurnRequest {
            conversation_id,
            user_prompt: prompt
                .parse::<UserPrompt>()
                .expect("the prompt should be valid"),
        }
    }

    fn new_store() -> EventStore {
        EventStore::new(
            std::env::temp_dir().join(format!("tog-turn-test-{}", uuid::Uuid::now_v7())),
        )
        .expect("the event store should be created")
    }

    fn event_kind(event: &ConversationEventRecord) -> &ConversationEventKind {
        let StoredConversationEventKind::Shared(kind) = &event.kind else {
            panic!("the record should be a conversation event");
        };
        kind
    }

    #[tokio::test]
    async fn a_turn_records_commands_outputs_and_completion() {
        let event_store = new_store();
        let inputs = Arc::new(Mutex::new(Vec::new()));
        let invocations = Arc::new(Mutex::new(Vec::new()));
        let source = model_source("test-provider", "test-model");
        let service = TurnService::new(
            event_store,
            Box::new(RecordingModelDriver {
                source,
                outputs: vec![
                    TestOutput::Communication("Thinking".to_owned()),
                    TestOutput::Assistant("Answer".to_owned()),
                ],
                inputs: Arc::clone(&inputs),
                invocations: Arc::clone(&invocations),
            }),
        );
        let mut conversation_id = None;

        service
            .execute(
                turn_request(None, "Question"),
                |identified| conversation_id = Some(identified),
                |_| Ok(()),
            )
            .await
            .expect("the turn should complete");

        let conversation_id = conversation_id.expect("the conversation should be identified");
        let log = service
            .event_store
            .load_conversation_log(conversation_id)
            .expect("the log should load");
        assert_eq!(log.len(), 7);
        assert!(matches!(
            &log[0].kind,
            StoredConversationEventKind::Shared(kind) if kind.is_command()
        ));
        assert!(matches!(
            event_kind(&log[1]),
            ConversationEventKind::User { .. }
        ));
        assert!(matches!(
            event_kind(&log[2]),
            ConversationEventKind::TurnRequested { .. }
        ));
        assert!(matches!(
            log[3].kind,
            StoredConversationEventKind::Extension(_)
        ));
        assert!(matches!(
            event_kind(&log[4]),
            ConversationEventKind::Communication { .. }
        ));
        assert!(matches!(
            event_kind(&log[5]),
            ConversationEventKind::Assistant { .. }
        ));
        assert!(matches!(
            event_kind(&log[6]),
            ConversationEventKind::TurnCompleted {
                outcome: TurnOutcome::Succeeded,
                ..
            }
        ));
        let invocation_id = invocations.lock().expect("the invocation list should lock")[0].1;
        assert!(matches!(
            event_kind(&log[4]),
            ConversationEventKind::Communication {
                invocation_id: found,
                ..
            } if *found == invocation_id
        ));
        assert!(matches!(
            event_kind(&log[5]),
            ConversationEventKind::Assistant {
                invocation_id: found,
                ..
            } if *found == invocation_id
        ));
        assert_eq!(inputs.lock().expect("the input list should lock").len(), 1);
    }

    #[tokio::test]
    async fn interface_failure_does_not_synthesize_problem_or_completion() {
        let service = TurnService::new(
            new_store(),
            Box::new(FailingModelDriver {
                source: model_source("test-provider", "test-model"),
            }),
        );
        let mut conversation_id = None;

        assert!(
            service
                .execute(
                    turn_request(None, "Question"),
                    |identified| conversation_id = Some(identified),
                    |_| Ok(()),
                )
                .await
                .is_err()
        );
        let log = service
            .event_store
            .load_conversation_log(conversation_id.expect("the conversation should be identified"))
            .expect("the log should load");
        assert_eq!(log.len(), 3);
    }

    #[tokio::test]
    async fn a_model_problem_completes_a_failed_turn_without_control_flow_failure() {
        let issue = ConversationProblem::Issue(
            ModelIssue::try_refusal("Refused.".to_owned()).expect("the issue should be valid"),
        );
        let service = TurnService::new(
            new_store(),
            Box::new(RecordingModelDriver {
                source: model_source("test-provider", "test-model"),
                outputs: vec![TestOutput::Problem(issue)],
                inputs: Arc::new(Mutex::new(Vec::new())),
                invocations: Arc::new(Mutex::new(Vec::new())),
            }),
        );
        let mut conversation_id = None;

        service
            .execute(
                turn_request(None, "Question"),
                |identified| conversation_id = Some(identified),
                |_| Ok(()),
            )
            .await
            .expect("a semantic model problem should not fail control flow");
        let log = service
            .event_store
            .load_conversation_log(conversation_id.expect("the conversation should be identified"))
            .expect("the log should load");
        assert!(matches!(
            event_kind(&log[4]),
            ConversationEventKind::Problem { .. }
        ));
        assert!(matches!(
            event_kind(&log[5]),
            ConversationEventKind::TurnCompleted {
                outcome: TurnOutcome::Failed,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn late_stream_failure_preserves_output_without_completion() {
        let service = TurnService::new(
            new_store(),
            Box::new(LateFailingModelDriver {
                source: model_source("test-provider", "test-model"),
            }),
        );
        let mut conversation_id = None;

        assert!(
            service
                .execute(
                    turn_request(None, "Question"),
                    |identified| conversation_id = Some(identified),
                    |_| Ok(()),
                )
                .await
                .is_err()
        );
        let log = service
            .event_store
            .load_conversation_log(conversation_id.expect("the conversation should be identified"))
            .expect("the log should load");
        assert!(matches!(
            event_kind(&log[3]),
            ConversationEventKind::Assistant { .. }
        ));
        assert_eq!(log.len(), 4);
    }
}
