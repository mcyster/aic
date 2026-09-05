use std::error::Error;

use futures_util::StreamExt;

use crate::conversation::{
    ConversationCommandId, ConversationEventKind, ConversationId, ConversationProblem,
    ConversationTurnId, InvalidConversationProblem, InvocationError, ModelDetails,
    ModelInvocationId, ModelSource, TurnOutcome, UserContent, UserPrompt,
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
            ConversationEventKind::UserMessageRequested {
                command_id: user_message_command_id,
                content: user_content.clone(),
            },
        )?;
        self.event_store.append_new_conversation_event(
            conversation_id,
            ConversationEventKind::User {
                caused_by: Some(user_message_command_id),
                content: user_content,
            },
        )?;
        conversation_identified(conversation_id);

        let source = self.model_driver.source().clone();
        let turn_id = ConversationTurnId::new();
        self.event_store.append_new_conversation_event(
            conversation_id,
            ConversationEventKind::TurnRequested {
                command_id: ConversationCommandId::new(),
                turn_id,
                model: source.clone(),
            },
        )?;
        let conversation = self.event_store.load_conversation(conversation_id)?;
        let invocation_id = ModelInvocationId::new();
        self.event_store.append_new_conversation_event(
            conversation_id,
            ConversationEventKind::ModelInvocationRequested {
                command_id: ConversationCommandId::new(),
                turn_id,
                invocation_id,
                model: source.clone(),
            },
        )?;
        report_progress(TurnProgress::InvocationStarted {
            model: source.model().as_str().to_owned(),
        })?;

        let mut conversation_events = match self
            .model_driver
            .invoke(&conversation, turn_id, invocation_id)
            .await
        {
            Ok(conversation_events) => conversation_events,
            Err(error) => {
                self.append_invocation_problem(
                    conversation_id,
                    turn_id,
                    invocation_id,
                    &source,
                    &error,
                    InvocationStage::BeforeStream,
                )?;
                self.append_turn_completed(conversation_id, turn_id, TurnOutcome::Failed)?;
                return Err(Box::new(error));
            }
        };

        let mut turn_failed = false;
        while let Some(conversation_event) = conversation_events.next().await {
            let conversation_kind = match conversation_event {
                Ok(conversation_kind) => conversation_kind,
                Err(error) => {
                    self.append_invocation_problem(
                        conversation_id,
                        turn_id,
                        invocation_id,
                        &source,
                        &error,
                        InvocationStage::DuringStream,
                    )?;
                    self.append_turn_completed(conversation_id, turn_id, TurnOutcome::Failed)?;
                    return Err(Box::new(error));
                }
            };
            match &conversation_kind {
                ConversationEventKind::Assistant { .. }
                | ConversationEventKind::Communication { .. } => {
                    self.event_store.append_new_conversation_event(
                        conversation_id,
                        conversation_kind.clone(),
                    )?;
                    report_progress(TurnProgress::EventCompleted {
                        event: conversation_kind,
                    })?;
                }
                ConversationEventKind::Problem { problem, .. } => {
                    let problem = problem.clone();
                    turn_failed = true;
                    self.event_store
                        .append_new_conversation_event(conversation_id, conversation_kind)?;
                    report_progress(TurnProgress::ProblemCompleted { problem })?;
                }
                _ => {
                    let error = ModelDriverError::InvalidResponse(
                        "the model driver returned an invalid conversation event".to_owned(),
                    );
                    self.append_invocation_problem(
                        conversation_id,
                        turn_id,
                        invocation_id,
                        &source,
                        &error,
                        InvocationStage::DuringStream,
                    )?;
                    self.append_turn_completed(conversation_id, turn_id, TurnOutcome::Failed)?;
                    return Err(Box::new(error));
                }
            }
        }

        self.append_turn_completed(
            conversation_id,
            turn_id,
            if turn_failed {
                TurnOutcome::Failed
            } else {
                TurnOutcome::Succeeded
            },
        )?;
        Ok(())
    }

    fn append_turn_completed(
        &self,
        conversation_id: ConversationId,
        turn_id: ConversationTurnId,
        outcome: TurnOutcome,
    ) -> TurnResultValue<()> {
        self.event_store.append_new_conversation_event(
            conversation_id,
            ConversationEventKind::TurnCompleted { turn_id, outcome },
        )?;
        Ok(())
    }

    fn append_invocation_problem(
        &self,
        conversation_id: ConversationId,
        turn_id: ConversationTurnId,
        invocation_id: ModelInvocationId,
        source: &ModelSource,
        error: &ModelDriverError,
        stage: InvocationStage,
    ) -> TurnResultValue<()> {
        let problem = invocation_problem(error, stage)?;
        let model = ModelDetails::new(source.clone(), None)?;
        self.event_store.append_new_conversation_event(
            conversation_id,
            ConversationEventKind::Problem {
                turn_id: Some(turn_id),
                invocation_id: Some(invocation_id),
                model: Some(model),
                problem,
            },
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

    use super::{TurnRequest, TurnService};
    use crate::conversation::{
        AssistantResponse, Conversation, ConversationEventKind, ConversationId,
        ConversationProblem, ConversationTurnId, InvocationError, ModelCommunication, ModelDetails,
        ModelEventImportance, ModelId, ModelInvocationId, ModelIssue, ModelSource, ProviderId,
        TurnOutcome, UserPrompt,
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

    impl ModelDriver for RecordingModelDriver {
        fn source(&self) -> &ModelSource {
            &self.source
        }

        fn invoke<'invoke>(
            &'invoke self,
            conversation: &'invoke Conversation,
            turn_id: ConversationTurnId,
            invocation_id: ModelInvocationId,
        ) -> BoxFuture<'invoke, Result<ModelOutputStream, ModelDriverError>> {
            self.inputs
                .lock()
                .expect("the model input list should lock")
                .push(conversation.clone());
            self.invocations
                .lock()
                .expect("the invocation list should lock")
                .push((turn_id, invocation_id));
            let outputs = self
                .outputs
                .iter()
                .map(|output| output_kind(output, &self.source, turn_id, invocation_id))
                .collect::<Vec<_>>();
            async move { Ok(stream::iter(outputs.into_iter().map(Ok)).boxed()) }.boxed()
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
            _invocation_id: ModelInvocationId,
        ) -> BoxFuture<'invoke, Result<ModelOutputStream, ModelDriverError>> {
            async { Err(ModelDriverError::Provider("failure".to_owned())) }.boxed()
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
            invocation_id: ModelInvocationId,
        ) -> BoxFuture<'invoke, Result<ModelOutputStream, ModelDriverError>> {
            let source = self.source.clone();
            async move {
                Ok(stream::iter(vec![
                    Ok(assistant_kind(
                        "Completed answer",
                        &source,
                        turn_id,
                        invocation_id,
                    )),
                    Err(ModelDriverError::Transport("late failure".to_owned())),
                ])
                .boxed())
            }
            .boxed()
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
        assert!(log[0].kind.is_command());
        assert!(matches!(log[1].kind, ConversationEventKind::User { .. }));
        assert!(matches!(
            log[2].kind,
            ConversationEventKind::TurnRequested { .. }
        ));
        assert!(matches!(
            log[3].kind,
            ConversationEventKind::ModelInvocationRequested { .. }
        ));
        assert!(matches!(
            log[4].kind,
            ConversationEventKind::Communication { .. }
        ));
        assert!(matches!(
            log[5].kind,
            ConversationEventKind::Assistant { .. }
        ));
        assert!(matches!(
            log[6].kind,
            ConversationEventKind::TurnCompleted {
                outcome: TurnOutcome::Succeeded,
                ..
            }
        ));
        let invocation_id = invocations.lock().expect("the invocation list should lock")[0].1;
        assert!(matches!(
            log[4].kind,
            ConversationEventKind::Communication {
                invocation_id: found,
                ..
            } if found == invocation_id
        ));
        assert!(matches!(
            log[5].kind,
            ConversationEventKind::Assistant {
                invocation_id: found,
                ..
            } if found == invocation_id
        ));
        assert_eq!(inputs.lock().expect("the input list should lock").len(), 1);
    }

    #[tokio::test]
    async fn failed_invocation_records_problem_and_failed_completion() {
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
        assert!(matches!(
            log[4].kind,
            ConversationEventKind::Problem {
                problem: ConversationProblem::Invocation(InvocationError::ProviderFailure { .. }),
                ..
            }
        ));
        assert!(matches!(
            log[5].kind,
            ConversationEventKind::TurnCompleted {
                outcome: TurnOutcome::Failed,
                ..
            }
        ));
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
        assert!(matches!(log[4].kind, ConversationEventKind::Problem { .. }));
        assert!(matches!(
            log[5].kind,
            ConversationEventKind::TurnCompleted {
                outcome: TurnOutcome::Failed,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn late_stream_failure_preserves_output_before_failed_completion() {
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
            log[4].kind,
            ConversationEventKind::Assistant { .. }
        ));
        assert!(matches!(log[5].kind, ConversationEventKind::Problem { .. }));
        assert!(matches!(
            log[6].kind,
            ConversationEventKind::TurnCompleted {
                outcome: TurnOutcome::Failed,
                ..
            }
        ));
    }
}
