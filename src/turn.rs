use std::error::Error;

use serde_json::{Value, json};

use crate::agent_run::{
    AgentRunEvent, AgentRunEventId, AgentRunId, ModelRequestSnapshot, ResponseVerbosity,
    StoredAgentRunEvent,
};
use crate::conversation::{ConversationEvent, ConversationId, ProjectionIdentity, UserPrompt};
use crate::openai::{
    OpenAiClient, OpenAiOutput, OpenAiResponseError, completed_response_text, semantic_input,
};
use crate::persistence::EventStore;

pub(crate) type TurnResultValue<T> = Result<T, Box<dyn Error>>;

pub(crate) struct TurnRequest {
    pub(crate) conversation_id: Option<ConversationId>,
    pub(crate) model: String,
    pub(crate) response_verbosity: ResponseVerbosity,
    pub(crate) user_prompt: UserPrompt,
}

pub(crate) struct TurnResult {
    pub(crate) assistant_text: String,
}

pub(crate) enum TurnProgress {
    ModelInvocationStarted { model: String },
    ProviderEventsReceived { count: usize },
}

pub(crate) struct TurnService {
    event_store: EventStore,
    openai_client: OpenAiClient,
}

impl TurnService {
    pub(crate) fn from_environment() -> TurnResultValue<Self> {
        Ok(Self {
            event_store: EventStore::from_environment()?,
            openai_client: OpenAiClient::from_environment()?,
        })
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
        self.recover_semantic_events(conversation.id)?;
        self.event_store.append_conversation_event(
            conversation.id,
            ConversationEvent::UserPrompt {
                text: request.user_prompt.text().to_owned(),
            },
        )?;

        let conversation_events = self.event_store.load_conversation_events(conversation.id)?;
        let previous_projection =
            conversation_events
                .iter()
                .rev()
                .nth(1)
                .and_then(|stored_event| match &stored_event.event {
                    ConversationEvent::AssistantResponse { projection, .. } => Some(*projection),
                    ConversationEvent::UserPrompt { .. } => None,
                });
        let previous_response_id = previous_projection
            .map(|projection| self.response_id_for_projection(projection))
            .transpose()?
            .flatten();
        let input = if previous_response_id.is_some() {
            json!([{ "role": "user", "content": request.user_prompt.text() }])
        } else {
            self.semantic_conversation_input(conversation.id)?
        };
        let model = request.model;
        let model_request = ModelRequestSnapshot {
            provider: "openai".to_owned(),
            model: model.clone(),
            response_verbosity: Some(request.response_verbosity),
            previous_response_id,
            input,
        };
        let invocation_result =
            self.execute_agent_run(conversation.id, &model_request, &mut report_progress);
        let (agent_run_id, output) = match invocation_result {
            Ok(result) => result,
            Err(error)
                if model_request.previous_response_id.is_some()
                    && error
                        .downcast_ref::<OpenAiResponseError>()
                        .is_some_and(OpenAiResponseError::permits_local_reconstruction) =>
            {
                let reconstruction_request = ModelRequestSnapshot {
                    provider: "openai".to_owned(),
                    model,
                    response_verbosity: Some(request.response_verbosity),
                    previous_response_id: None,
                    input: self.semantic_conversation_input(conversation.id)?,
                };
                self.execute_agent_run(
                    conversation.id,
                    &reconstruction_request,
                    &mut report_progress,
                )?
            }
            Err(error) => return Err(error),
        };

        self.event_store.append_conversation_event(
            conversation.id,
            ConversationEvent::AssistantResponse {
                text: output.assistant_text.clone(),
                projection: ProjectionIdentity {
                    source_run_id: agent_run_id,
                    source_run_event_id: output.completion_event_id,
                    output_index: 0,
                },
            },
        )?;
        self.event_store.append_agent_run_event(
            agent_run_id,
            AgentRunEvent::RunCompleted {
                response_id: output.response_id,
            },
        )?;

        Ok(TurnResult {
            assistant_text: output.assistant_text,
        })
    }

    fn execute_agent_run(
        &self,
        conversation_id: ConversationId,
        model_request: &ModelRequestSnapshot,
        report_progress: &mut impl FnMut(TurnProgress),
    ) -> TurnResultValue<(AgentRunId, OpenAiOutput)> {
        let agent_run = self.event_store.create_agent_run(conversation_id)?;
        self.event_store.append_agent_run_event(
            agent_run.id,
            AgentRunEvent::RunStarted {
                request: model_request.clone(),
            },
        )?;

        report_progress(TurnProgress::ModelInvocationStarted {
            model: model_request.model.clone(),
        });
        let mut provider_event_count = 0_usize;
        let invocation_result = self.openai_client.invoke(
            model_request,
            |event_type, payload| -> TurnResultValue<AgentRunEventId> {
                let stored_event = self.event_store.append_agent_run_event(
                    agent_run.id,
                    AgentRunEvent::ModelProviderEvent {
                        provider: "openai".to_owned(),
                        model: model_request.model.clone(),
                        event_type,
                        payload,
                    },
                )?;
                provider_event_count = provider_event_count.saturating_add(1);
                if provider_event_count == 1 || provider_event_count.is_multiple_of(100) {
                    report_progress(TurnProgress::ProviderEventsReceived {
                        count: provider_event_count,
                    });
                }
                Ok(stored_event.id)
            },
        );
        let output = match invocation_result {
            Ok(output) => output,
            Err(error) => {
                self.event_store.append_agent_run_event(
                    agent_run.id,
                    AgentRunEvent::RunFailed {
                        message: error.to_string(),
                    },
                )?;
                return Err(error);
            }
        };
        Ok((agent_run.id, output))
    }

    fn recover_semantic_events(&self, conversation_id: ConversationId) -> TurnResultValue<()> {
        for agent_run in self.event_store.load_agent_runs(conversation_id)? {
            let events = self.event_store.load_agent_run_events(agent_run.id)?;
            if let Some((source_run_event_id, assistant_text)) = project_assistant(&events) {
                self.event_store.append_conversation_event(
                    conversation_id,
                    ConversationEvent::AssistantResponse {
                        text: assistant_text,
                        projection: ProjectionIdentity {
                            source_run_id: agent_run.id,
                            source_run_event_id,
                            output_index: 0,
                        },
                    },
                )?;
            }
        }
        Ok(())
    }

    fn response_id_for_projection(
        &self,
        projection: ProjectionIdentity,
    ) -> TurnResultValue<Option<String>> {
        for stored_event in self
            .event_store
            .load_agent_run_events(projection.source_run_id)?
        {
            if stored_event.id != projection.source_run_event_id {
                continue;
            }
            if let AgentRunEvent::ModelProviderEvent {
                event_type,
                payload,
                ..
            } = stored_event.event
                && event_type == "response.completed"
            {
                return Ok(payload
                    .get("response")
                    .and_then(|response| response.get("id"))
                    .and_then(Value::as_str)
                    .map(str::to_owned));
            }
        }
        Ok(None)
    }

    fn semantic_conversation_input(
        &self,
        conversation_id: ConversationId,
    ) -> TurnResultValue<Value> {
        let messages = self
            .event_store
            .load_conversation_events(conversation_id)?
            .into_iter()
            .map(|stored_event| match stored_event.event {
                ConversationEvent::UserPrompt { text } => ("user".to_owned(), text),
                ConversationEvent::AssistantResponse { text, .. } => ("assistant".to_owned(), text),
            });
        Ok(semantic_input(messages))
    }
}

fn project_assistant(events: &[StoredAgentRunEvent]) -> Option<(AgentRunEventId, String)> {
    let mut streamed_text = String::new();
    let mut completed_text = None;
    let mut completion_event_id = None;
    for stored_event in events {
        let AgentRunEvent::ModelProviderEvent {
            event_type,
            payload,
            ..
        } = &stored_event.event
        else {
            continue;
        };
        match event_type.as_str() {
            "response.output_text.delta" => {
                if let Some(delta) = payload.get("delta").and_then(Value::as_str) {
                    streamed_text.push_str(delta);
                }
            }
            "response.refusal.delta" => {
                if let Some(delta) = payload.get("delta").and_then(Value::as_str) {
                    streamed_text.push_str(delta);
                }
            }
            "response.output_text.done" => {
                completed_text = payload
                    .get("text")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            "response.refusal.done" => {
                completed_text = payload
                    .get("refusal")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            "response.completed" => {
                if completed_text.is_none() {
                    completed_text = completed_response_text(payload);
                }
                completion_event_id = Some(stored_event.id);
            }
            _ => {}
        }
    }
    let assistant_text = if streamed_text.is_empty() {
        completed_text?
    } else {
        streamed_text
    };
    Some((completion_event_id?, assistant_text))
}
