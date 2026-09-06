use std::collections::HashSet;
use std::error::Error;

use futures_util::StreamExt;

use crate::conversation::{
    ConversationCommandId, ConversationEvent, ConversationFact, ConversationId,
    ConversationProblem, ConversationRequest, ConversationTurnId, DriverConversationEvent,
    DriverConversationFact, UserContent, UserPrompt,
};
use crate::model_driver::{ModelDriver, ModelDriverError, ModelDriverRequest};
use crate::persistence::EventStore;

pub(crate) type TurnResultValue<T> = Result<T, Box<dyn Error>>;

pub(crate) enum TurnProgress {
    InvocationStarted { model: String },
    EventCompleted { event: ConversationFact },
    ProblemCompleted { problem: ConversationProblem },
}

pub(crate) struct ConversationSession {
    event_store: EventStore,
    model_driver: Box<dyn ModelDriver>,
}

impl ConversationSession {
    pub(crate) fn new(event_store: EventStore, model_driver: Box<dyn ModelDriver>) -> Self {
        Self {
            event_store,
            model_driver,
        }
    }

    pub(crate) fn add_user_request(
        &self,
        conversation_id: Option<ConversationId>,
        user_prompt: UserPrompt,
    ) -> TurnResultValue<(ConversationId, ConversationCommandId)> {
        let conversation_id = match conversation_id {
            Some(conversation_id) => {
                self.event_store.load_conversation(conversation_id)?;
                conversation_id
            }
            None => ConversationId::new(),
        };
        let command_id = ConversationCommandId::new();
        self.event_store.append_new_conversation_event(
            conversation_id,
            ConversationEvent::Request(ConversationRequest::UserMessageRequested {
                command_id,
                content: vec![UserContent::Text(user_prompt.text().to_owned())],
            }),
        )?;
        Ok((conversation_id, command_id))
    }

    pub(crate) async fn invoke(
        &self,
        conversation_id: ConversationId,
        mut report_progress: impl FnMut(TurnProgress) -> TurnResultValue<()>,
    ) -> TurnResultValue<()> {
        let turn_id = ConversationTurnId::new();
        self.event_store.append_new_conversation_event(
            conversation_id,
            ConversationEvent::Request(ConversationRequest::TurnRequested {
                command_id: ConversationCommandId::new(),
                turn_id,
            }),
        )?;
        let conversation = self.event_store.load_conversation(conversation_id)?;
        let pending_user_requests = conversation.pending_user_requests();
        let source = self.model_driver.source().clone();
        report_progress(TurnProgress::InvocationStarted {
            model: source.model().as_str().to_owned(),
        })?;

        let driver_request = ModelDriverRequest::new(&conversation, pending_user_requests, turn_id);
        let mut output_stream = self.model_driver.invoke(driver_request).await?;
        let mut turn_completed = false;
        let mut accepted_request_ids = HashSet::new();

        while let Some(output) = output_stream.next().await {
            let output = output?;
            if turn_completed {
                return Err(Box::new(ModelDriverError::OutputAfterCompletion {
                    event_type: driver_event_type(&output),
                }));
            }
            match output {
                DriverConversationEvent::Command(event) => {
                    self.event_store.append_new_conversation_event(
                        conversation_id,
                        ConversationEvent::Driver(DriverConversationEvent::Command(event)),
                    )?;
                }
                DriverConversationEvent::Fact(DriverConversationFact::Extension(event)) => {
                    self.event_store.append_new_conversation_event(
                        conversation_id,
                        ConversationEvent::Driver(DriverConversationEvent::Fact(
                            DriverConversationFact::Extension(event),
                        )),
                    )?;
                }
                DriverConversationEvent::Fact(DriverConversationFact::Shared(fact)) => {
                    match &fact {
                        ConversationFact::User { caused_by, .. } => {
                            let Some(command_id) = caused_by else {
                                return Err(Box::new(ModelDriverError::MissingTurnIdentity));
                            };
                            if !pending_request_ids(&conversation).contains(command_id)
                                || !accepted_request_ids.insert(*command_id)
                            {
                                return Err(Box::new(ModelDriverError::DisallowedEventKind {
                                    event_type: "user".to_owned(),
                                }));
                            }
                            self.append_shared_fact(conversation_id, fact)?;
                        }
                        ConversationFact::Assistant {
                            turn_id: fact_turn_id,
                            ..
                        }
                        | ConversationFact::Communication {
                            turn_id: fact_turn_id,
                            ..
                        }
                        | ConversationFact::TurnCompleted {
                            turn_id: fact_turn_id,
                            ..
                        } => {
                            ensure_turn_id(*fact_turn_id, &turn_id)?;
                            if matches!(fact, ConversationFact::TurnCompleted { .. }) {
                                turn_completed = true;
                            }
                            self.report_shared_fact(conversation_id, fact, &mut report_progress)?;
                        }
                        ConversationFact::Problem {
                            turn_id: problem_turn_id,
                            ..
                        } => {
                            ensure_optional_turn_id(*problem_turn_id, &turn_id)?;
                            self.report_shared_fact(conversation_id, fact, &mut report_progress)?;
                        }
                    }
                }
            }
        }

        if !turn_completed {
            return Err(Box::new(ModelDriverError::IncompleteTurn));
        }
        Ok(())
    }

    fn append_shared_fact(
        &self,
        conversation_id: ConversationId,
        fact: ConversationFact,
    ) -> TurnResultValue<()> {
        self.event_store.append_new_conversation_event(
            conversation_id,
            ConversationEvent::Driver(DriverConversationEvent::Fact(
                DriverConversationFact::Shared(fact),
            )),
        )?;
        Ok(())
    }

    fn report_shared_fact(
        &self,
        conversation_id: ConversationId,
        fact: ConversationFact,
        report_progress: &mut impl FnMut(TurnProgress) -> TurnResultValue<()>,
    ) -> TurnResultValue<()> {
        match &fact {
            ConversationFact::Assistant { .. } | ConversationFact::Communication { .. } => {
                let progress = TurnProgress::EventCompleted {
                    event: fact.clone(),
                };
                self.append_shared_fact(conversation_id, fact)?;
                report_progress(progress)?;
            }
            ConversationFact::Problem { problem, .. } => {
                let progress = TurnProgress::ProblemCompleted {
                    problem: problem.clone(),
                };
                self.append_shared_fact(conversation_id, fact)?;
                report_progress(progress)?;
            }
            ConversationFact::User { .. } | ConversationFact::TurnCompleted { .. } => {
                self.append_shared_fact(conversation_id, fact)?;
            }
        }
        Ok(())
    }
}

fn pending_request_ids(
    conversation: &crate::conversation::Conversation,
) -> HashSet<ConversationCommandId> {
    conversation
        .pending_user_requests()
        .iter()
        .filter_map(|request| request.user_message().map(|(command_id, _)| command_id))
        .collect()
}

fn ensure_turn_id(
    actual_turn_id: ConversationTurnId,
    expected_turn_id: &ConversationTurnId,
) -> Result<(), ModelDriverError> {
    if actual_turn_id != *expected_turn_id {
        return Err(ModelDriverError::WrongTurnIdentity {
            expected: *expected_turn_id,
            actual: actual_turn_id,
        });
    }
    Ok(())
}

fn ensure_optional_turn_id(
    actual_turn_id: Option<ConversationTurnId>,
    expected_turn_id: &ConversationTurnId,
) -> Result<(), ModelDriverError> {
    actual_turn_id
        .map(|actual_turn_id| ensure_turn_id(actual_turn_id, expected_turn_id))
        .unwrap_or(Ok(()))
}

fn driver_event_type(event: &DriverConversationEvent) -> String {
    match event {
        DriverConversationEvent::Command(_) => "driver_command".to_owned(),
        DriverConversationEvent::Fact(DriverConversationFact::Shared(fact)) => match fact {
            ConversationFact::User { .. } => "user".to_owned(),
            ConversationFact::Assistant { .. } => "assistant".to_owned(),
            ConversationFact::Communication { .. } => "communication".to_owned(),
            ConversationFact::Problem { .. } => "problem".to_owned(),
            ConversationFact::TurnCompleted { .. } => "turn_completed".to_owned(),
        },
        DriverConversationEvent::Fact(DriverConversationFact::Extension(_)) => {
            "driver_fact".to_owned()
        }
    }
}
