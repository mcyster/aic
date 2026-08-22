use std::collections::VecDeque;
use std::str::FromStr;

use futures_util::future::BoxFuture;
use futures_util::stream::{self, BoxStream};
use futures_util::{FutureExt, StreamExt};
use reqwest::Client;
use reqwest::StatusCode;
use serde_json::{Map, Value, json};

use crate::conversation::{
    AssistantResponse, Conversation, ConversationEventKind, InvalidAssistantResponse,
    InvalidModelCommunication, ModelCommunication, ModelEvent, ModelEventImportance, ModelId,
    ModelSource, ProviderId, UserContent,
};
use crate::model_driver::{ModelDriver, ModelDriverError, ModelEventStream};

type ResponseByteStream = BoxStream<'static, Result<Vec<u8>, ModelDriverError>>;

pub(crate) struct OpenAiModelDriver {
    http_client: Client,
    api_key: String,
    responses_url: String,
    source: ModelSource,
}

impl OpenAiModelDriver {
    pub(crate) fn from_environment(model: ModelId) -> Result<Self, ModelDriverError> {
        let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
            ModelDriverError::Authentication("OPENAI_API_KEY must be set".to_owned())
        })?;
        let base_url = std::env::var("TOG_OPENAI_BASE_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1".to_owned());
        Ok(Self {
            http_client: Client::new(),
            api_key,
            responses_url: format!("{}/responses", base_url.trim_end_matches('/')),
            source: ModelSource::new(
                ProviderId::from_str("openai")
                    .expect("the OpenAI provider identifier should be valid"),
                model,
            ),
        })
    }
}

impl ModelDriver for OpenAiModelDriver {
    fn source(&self) -> &ModelSource {
        &self.source
    }

    fn invoke<'invoke>(
        &'invoke self,
        conversation: &'invoke Conversation,
    ) -> BoxFuture<'invoke, Result<ModelEventStream, ModelDriverError>> {
        let mut request_body = Map::new();
        request_body.insert(
            "model".to_owned(),
            Value::String(self.source.model().as_str().to_owned()),
        );
        request_body.insert("input".to_owned(), semantic_input(conversation));
        request_body.insert("reasoning".to_owned(), json!({ "summary": "auto" }));
        request_body.insert("stream".to_owned(), Value::Bool(true));
        request_body.insert("store".to_owned(), Value::Bool(true));

        let request = self
            .http_client
            .post(&self.responses_url)
            .bearer_auth(&self.api_key)
            .json(&request_body)
            .build();
        let http_client = self.http_client.clone();

        async move {
            let request =
                request.map_err(|error| ModelDriverError::InvalidRequest(error.to_string()))?;
            let response = http_client
                .execute(request)
                .await
                .map_err(|error| ModelDriverError::Transport(error.to_string()))?;
            let response_status = response.status();
            if !response_status.is_success() {
                let response_body = response
                    .text()
                    .await
                    .map_err(|error| ModelDriverError::Transport(error.to_string()))?;
                return Err(classify_response_error(response_status, response_body));
            }

            let response_bytes = response
                .bytes_stream()
                .map(|result| {
                    result
                        .map(|bytes| bytes.to_vec())
                        .map_err(|error| ModelDriverError::Transport(error.to_string()))
                })
                .boxed();
            Ok(model_event_stream(response_bytes))
        }
        .boxed()
    }
}

fn semantic_input(conversation: &Conversation) -> Value {
    Value::Array(
        conversation
            .events()
            .iter()
            .filter_map(|conversation_event| match &conversation_event.kind {
                ConversationEventKind::User { content } => {
                    let text = content
                        .iter()
                        .map(|content| match content {
                            UserContent::Text(text) => text.as_str(),
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    Some(json!({ "role": "user", "content": text }))
                }
                ConversationEventKind::Model {
                    event: ModelEvent::AssistantResponse(response),
                    ..
                } => Some(json!({ "role": "assistant", "content": response.message() })),
                ConversationEventKind::Model { .. } => None,
            })
            .collect(),
    )
}

fn classify_response_error(status: StatusCode, body: String) -> ModelDriverError {
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ModelDriverError::Authentication(body),
        StatusCode::TOO_MANY_REQUESTS => ModelDriverError::RateLimited(body),
        StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND | StatusCode::UNPROCESSABLE_ENTITY => {
            ModelDriverError::InvalidRequest(body)
        }
        _ => ModelDriverError::Provider(format!("OpenAI Responses returned {status}: {body}")),
    }
}

#[derive(Default)]
struct ResponseState {
    assistant_outputs: Vec<AccumulatedText>,
    reasoning_outputs: Vec<AccumulatedText>,
    reasoning_summaries: Vec<AccumulatedText>,
    completed: bool,
}

struct AccumulatedText {
    key: String,
    streamed_text: String,
    completed_text: Option<String>,
    emitted: bool,
}

struct ServerSentEvent {
    name: Option<String>,
    data: String,
}

#[derive(Default)]
struct ServerSentEventDecoder {
    bytes: Vec<u8>,
    event_name: Option<String>,
    data_lines: Vec<String>,
    events: VecDeque<ServerSentEvent>,
}

impl ServerSentEventDecoder {
    fn push(&mut self, bytes: &[u8]) -> Result<(), ModelDriverError> {
        self.bytes.extend_from_slice(bytes);
        while let Some(newline_position) = self.bytes.iter().position(|byte| *byte == b'\n') {
            let mut line = self.bytes.drain(..=newline_position).collect::<Vec<_>>();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            self.process_line(line)?;
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<(), ModelDriverError> {
        if !self.bytes.is_empty() {
            let mut line = std::mem::take(&mut self.bytes);
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            self.process_line(line)?;
        }
        self.dispatch_event();
        Ok(())
    }

    fn process_line(&mut self, line: Vec<u8>) -> Result<(), ModelDriverError> {
        let line = String::from_utf8(line)
            .map_err(|error| ModelDriverError::InvalidResponse(error.to_string()))?;
        if line.is_empty() {
            self.dispatch_event();
            return Ok(());
        }
        if line.starts_with(':') {
            return Ok(());
        }

        let (field, value) = line
            .split_once(':')
            .map_or((line.as_str(), ""), |(field, value)| {
                (field, value.strip_prefix(' ').unwrap_or(value))
            });
        match field {
            "event" => self.event_name = Some(value.to_owned()),
            "data" => self.data_lines.push(value.to_owned()),
            _ => {}
        }
        Ok(())
    }

    fn dispatch_event(&mut self) {
        if !self.data_lines.is_empty() {
            self.events.push_back(ServerSentEvent {
                name: self.event_name.take(),
                data: self.data_lines.join("\n"),
            });
            self.data_lines.clear();
        } else {
            self.event_name = None;
        }
    }
}

struct OpenAiStreamState {
    response_bytes: ResponseByteStream,
    decoder: ServerSentEventDecoder,
    response: ResponseState,
    model_events: VecDeque<ModelEvent>,
    response_bytes_finished: bool,
    terminated: bool,
}

fn model_event_stream(response_bytes: ResponseByteStream) -> ModelEventStream {
    let state = OpenAiStreamState {
        response_bytes,
        decoder: ServerSentEventDecoder::default(),
        response: ResponseState::default(),
        model_events: VecDeque::new(),
        response_bytes_finished: false,
        terminated: false,
    };

    stream::unfold(state, |mut state| async move {
        loop {
            if let Some(model_event) = state.model_events.pop_front() {
                return Some((Ok(model_event), state));
            }
            if state.terminated {
                return None;
            }
            if let Some(server_sent_event) = state.decoder.events.pop_front() {
                match process_event(server_sent_event, &mut state.response) {
                    Ok(ProcessEventResult::Events(model_events)) => {
                        state.model_events.extend(model_events);
                    }
                    Ok(ProcessEventResult::Done) => {
                        state.decoder.events.clear();
                        state.response_bytes_finished = true;
                    }
                    Err(error) => {
                        state.terminated = true;
                        return Some((Err(error), state));
                    }
                }
                continue;
            }
            if state.response_bytes_finished {
                state.terminated = true;
                if state.response.completed {
                    return None;
                }
                return Some((
                    Err(ModelDriverError::InvalidResponse(
                        "response.completed was not received".to_owned(),
                    )),
                    state,
                ));
            }

            match state.response_bytes.next().await {
                Some(Ok(bytes)) => {
                    if let Err(error) = state.decoder.push(&bytes) {
                        state.terminated = true;
                        return Some((Err(error), state));
                    }
                }
                Some(Err(error)) => {
                    state.terminated = true;
                    return Some((Err(error), state));
                }
                None => {
                    if let Err(error) = state.decoder.finish() {
                        state.terminated = true;
                        return Some((Err(error), state));
                    }
                    state.response_bytes_finished = true;
                }
            }
        }
    })
    .boxed()
}

enum ProcessEventResult {
    Events(Vec<ModelEvent>),
    Done,
}

fn process_event(
    server_sent_event: ServerSentEvent,
    response_state: &mut ResponseState,
) -> Result<ProcessEventResult, ModelDriverError> {
    if server_sent_event.data == "[DONE]" {
        return Ok(ProcessEventResult::Done);
    }

    let payload: Value = serde_json::from_str(&server_sent_event.data)
        .map_err(|error| ModelDriverError::InvalidResponse(error.to_string()))?;
    let payload_event_type = match payload.get("type") {
        Some(Value::String(event_type)) => Some(event_type.clone()),
        Some(_) => {
            return Err(ModelDriverError::InvalidResponse(
                "an OpenAI stream event contained a non-string type".to_owned(),
            ));
        }
        None => None,
    };
    if let (Some(payload_event_type), Some(server_sent_event_name)) =
        (&payload_event_type, &server_sent_event.name)
        && payload_event_type != server_sent_event_name
    {
        return Err(ModelDriverError::InvalidResponse(format!(
            "OpenAI stream event type {server_sent_event_name} did not match payload type {payload_event_type}"
        )));
    }
    let event_type = payload_event_type
        .or(server_sent_event.name)
        .unwrap_or_else(|| "unknown".to_owned());

    if response_state.completed
        && !matches!(
            event_type.as_str(),
            "error" | "response.failed" | "response.completed"
        )
    {
        return Err(ModelDriverError::InvalidResponse(format!(
            "OpenAI stream emitted {event_type} after response.completed"
        )));
    }

    let model_events = match event_type.as_str() {
        "response.output_text.delta" | "response.refusal.delta" => {
            let key = semantic_output_key(&payload, &["output_index", "content_index"])?;
            append_delta(
                &payload,
                accumulated_text(&mut response_state.assistant_outputs, key)?,
            )?;
            Vec::new()
        }
        "response.output_text.done" => {
            let key = semantic_output_key(&payload, &["output_index", "content_index"])?;
            complete_text(
                &payload,
                "text",
                accumulated_text(&mut response_state.assistant_outputs, key.clone())?,
            )?;
            emit_assistant_response(&mut response_state.assistant_outputs, &key)?
                .into_iter()
                .collect()
        }
        "response.refusal.done" => {
            let key = semantic_output_key(&payload, &["output_index", "content_index"])?;
            complete_text(
                &payload,
                "refusal",
                accumulated_text(&mut response_state.assistant_outputs, key.clone())?,
            )?;
            emit_assistant_response(&mut response_state.assistant_outputs, &key)?
                .into_iter()
                .collect()
        }
        "response.reasoning_text.delta" => {
            let key = semantic_output_key(&payload, &["output_index", "content_index"])?;
            append_delta(
                &payload,
                accumulated_text(&mut response_state.reasoning_outputs, key)?,
            )?;
            Vec::new()
        }
        "response.reasoning_text.done" => {
            let key = semantic_output_key(&payload, &["output_index", "content_index"])?;
            complete_text(
                &payload,
                "text",
                accumulated_text(&mut response_state.reasoning_outputs, key.clone())?,
            )?;
            emit_reasoning(&mut response_state.reasoning_outputs, &key)?
                .into_iter()
                .collect()
        }
        "response.reasoning_summary_text.delta" => {
            let key = semantic_output_key(&payload, &["output_index", "summary_index"])?;
            append_delta(
                &payload,
                accumulated_text(&mut response_state.reasoning_summaries, key)?,
            )?;
            Vec::new()
        }
        "response.reasoning_summary_text.done" => {
            let key = semantic_output_key(&payload, &["output_index", "summary_index"])?;
            complete_text(
                &payload,
                "text",
                accumulated_text(&mut response_state.reasoning_summaries, key.clone())?,
            )?;
            emit_reasoning_summary(&mut response_state.reasoning_summaries, &key)?
                .into_iter()
                .collect()
        }
        "response.completed" => complete_response(response_state, &payload)?,
        "error" | "response.failed" => {
            return Err(ModelDriverError::Provider(format!(
                "OpenAI stream emitted {event_type}: {payload}"
            )));
        }
        _ => Vec::new(),
    };
    Ok(ProcessEventResult::Events(model_events))
}

fn complete_response(
    response_state: &mut ResponseState,
    payload: &Value,
) -> Result<Vec<ModelEvent>, ModelDriverError> {
    if response_state.completed {
        return Err(ModelDriverError::InvalidResponse(
            "response.completed was received more than once".to_owned(),
        ));
    }
    if !payload.get("response").is_some_and(Value::is_object) {
        return Err(ModelDriverError::InvalidResponse(
            "response.completed did not contain an OpenAI response object".to_owned(),
        ));
    }

    let mut model_events = Vec::new();
    model_events.extend(emit_remaining_reasoning(
        &mut response_state.reasoning_outputs,
    )?);
    model_events.extend(emit_remaining_reasoning_summaries(
        &mut response_state.reasoning_summaries,
    )?);
    model_events.extend(emit_remaining_assistant_responses(
        &mut response_state.assistant_outputs,
    )?);
    if !response_state
        .assistant_outputs
        .iter()
        .any(|output| output.emitted)
    {
        let assistant_text = completed_response_text(payload)?.ok_or_else(|| {
            ModelDriverError::InvalidResponse(
                "the completed response contained no model message".to_owned(),
            )
        })?;
        model_events.push(assistant_response(assistant_text)?);
    }
    response_state.completed = true;
    Ok(model_events)
}

fn emit_reasoning(
    reasoning_outputs: &mut [AccumulatedText],
    key: &str,
) -> Result<Option<ModelEvent>, ModelDriverError> {
    let output = un_emitted_text(reasoning_outputs, key)?;
    let Some(reasoning_text) = preferred_text(&output.streamed_text, &output.completed_text) else {
        return Ok(None);
    };
    let model_event =
        model_communication(reasoning_text, "reasoning", ModelEventImportance::Detailed)?;
    output.emitted = true;
    Ok(Some(model_event))
}

fn emit_reasoning_summary(
    reasoning_summaries: &mut [AccumulatedText],
    key: &str,
) -> Result<Option<ModelEvent>, ModelDriverError> {
    let output = un_emitted_text(reasoning_summaries, key)?;
    let Some(reasoning_summary) = preferred_text(&output.streamed_text, &output.completed_text)
    else {
        return Ok(None);
    };
    let model_event = model_communication(
        reasoning_summary,
        "reasoning_summary",
        ModelEventImportance::Interesting,
    )?;
    output.emitted = true;
    Ok(Some(model_event))
}

fn emit_assistant_response(
    assistant_outputs: &mut [AccumulatedText],
    key: &str,
) -> Result<Option<ModelEvent>, ModelDriverError> {
    let output = un_emitted_text(assistant_outputs, key)?;
    let assistant_text = preferred_text(&output.streamed_text, &output.completed_text);
    let Some(assistant_text) = assistant_text else {
        return Ok(None);
    };
    let model_event = assistant_response(assistant_text)?;
    output.emitted = true;
    Ok(Some(model_event))
}

fn emit_remaining_reasoning(
    outputs: &mut [AccumulatedText],
) -> Result<Vec<ModelEvent>, ModelDriverError> {
    let keys = un_emitted_keys(outputs);
    keys.into_iter()
        .filter_map(|key| emit_reasoning(outputs, &key).transpose())
        .collect()
}

fn emit_remaining_reasoning_summaries(
    outputs: &mut [AccumulatedText],
) -> Result<Vec<ModelEvent>, ModelDriverError> {
    let keys = un_emitted_keys(outputs);
    keys.into_iter()
        .filter_map(|key| emit_reasoning_summary(outputs, &key).transpose())
        .collect()
}

fn emit_remaining_assistant_responses(
    outputs: &mut [AccumulatedText],
) -> Result<Vec<ModelEvent>, ModelDriverError> {
    let keys = un_emitted_keys(outputs);
    keys.into_iter()
        .filter_map(|key| emit_assistant_response(outputs, &key).transpose())
        .collect()
}

fn assistant_response(message: String) -> Result<ModelEvent, ModelDriverError> {
    AssistantResponse::new(message, Map::new())
        .map(ModelEvent::AssistantResponse)
        .map_err(invalid_assistant_response)
}

fn semantic_output_key(payload: &Value, indexes: &[&str]) -> Result<String, ModelDriverError> {
    let mut key_parts = Vec::new();
    for index_name in indexes {
        if let Some(index) = payload.get(index_name) {
            let index = index.as_u64().ok_or_else(|| {
                ModelDriverError::InvalidResponse(format!(
                    "an OpenAI semantic event contained an invalid {index_name}"
                ))
            })?;
            key_parts.push(format!("{index_name}={index}"));
        }
    }
    if !key_parts.is_empty() && key_parts.len() != indexes.len() {
        return Err(ModelDriverError::InvalidResponse(
            "an OpenAI semantic event contained incomplete output indexes".to_owned(),
        ));
    }
    if key_parts.len() == indexes.len() {
        return Ok(key_parts.join(";"));
    }

    match payload.get("item_id") {
        Some(Value::String(item_id)) => Ok(format!("item_id={item_id}")),
        Some(_) => Err(ModelDriverError::InvalidResponse(
            "an OpenAI semantic event contained a non-string item_id".to_owned(),
        )),
        None => Ok("default".to_owned()),
    }
}

fn accumulated_text(
    outputs: &mut Vec<AccumulatedText>,
    key: String,
) -> Result<&mut AccumulatedText, ModelDriverError> {
    if let Some(position) = outputs.iter().position(|output| output.key == key) {
        return Ok(&mut outputs[position]);
    }
    if (key == "default" && !outputs.is_empty())
        || (key != "default" && outputs.iter().any(|output| output.key == "default"))
    {
        return Err(ModelDriverError::InvalidResponse(
            "OpenAI semantic output identity changed while streaming".to_owned(),
        ));
    }
    outputs.push(AccumulatedText {
        key,
        streamed_text: String::new(),
        completed_text: None,
        emitted: false,
    });
    Ok(outputs
        .last_mut()
        .expect("the accumulated output was just added"))
}

fn un_emitted_text<'a>(
    outputs: &'a mut [AccumulatedText],
    key: &str,
) -> Result<&'a mut AccumulatedText, ModelDriverError> {
    let output = outputs
        .iter_mut()
        .find(|output| output.key == key)
        .expect("the accumulated output should exist");
    if output.emitted {
        return Err(ModelDriverError::InvalidResponse(format!(
            "OpenAI emitted semantic output {key} more than once"
        )));
    }
    Ok(output)
}

fn un_emitted_keys(outputs: &[AccumulatedText]) -> Vec<String> {
    outputs
        .iter()
        .filter(|output| !output.emitted)
        .map(|output| output.key.clone())
        .collect()
}

fn complete_text(
    payload: &Value,
    field: &str,
    output: &mut AccumulatedText,
) -> Result<(), ModelDriverError> {
    if output.emitted {
        return Err(ModelDriverError::InvalidResponse(format!(
            "OpenAI emitted semantic output {} more than once",
            output.key
        )));
    }
    output.completed_text = text_field(payload, field)?;
    Ok(())
}

fn model_communication(
    message: String,
    subtype: &str,
    importance: ModelEventImportance,
) -> Result<ModelEvent, ModelDriverError> {
    ModelCommunication::new(message, importance, subtype.to_owned(), Map::new())
        .map(ModelEvent::Communication)
        .map_err(invalid_model_communication)
}

fn invalid_assistant_response(error: InvalidAssistantResponse) -> ModelDriverError {
    ModelDriverError::InvalidResponse(error.to_string())
}

fn invalid_model_communication(error: InvalidModelCommunication) -> ModelDriverError {
    ModelDriverError::InvalidResponse(error.to_string())
}

fn append_delta(payload: &Value, output: &mut AccumulatedText) -> Result<(), ModelDriverError> {
    if output.emitted {
        return Err(ModelDriverError::InvalidResponse(format!(
            "OpenAI emitted a delta after completing semantic output {}",
            output.key
        )));
    }
    let delta = payload
        .get("delta")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ModelDriverError::InvalidResponse(
                "an OpenAI delta event did not contain a string delta".to_owned(),
            )
        })?;
    output.streamed_text.push_str(delta);
    Ok(())
}

fn text_field(payload: &Value, field: &str) -> Result<Option<String>, ModelDriverError> {
    let Some(value) = payload.get(field) else {
        return Ok(None);
    };
    value
        .as_str()
        .map(|text| Some(text.to_owned()))
        .ok_or_else(|| {
            ModelDriverError::InvalidResponse(format!(
                "an OpenAI completion event contained a non-string {field} field"
            ))
        })
}

fn preferred_text(streamed_text: &str, completed_text: &Option<String>) -> Option<String> {
    completed_text
        .clone()
        .or_else(|| (!streamed_text.is_empty()).then(|| streamed_text.to_owned()))
}

fn completed_response_text(payload: &Value) -> Result<Option<String>, ModelDriverError> {
    let Some(output_value) = payload
        .get("response")
        .and_then(|response| response.get("output"))
    else {
        return Ok(None);
    };
    let output = output_value.as_array().ok_or_else(|| {
        ModelDriverError::InvalidResponse(
            "the completed OpenAI response contained non-array output".to_owned(),
        )
    })?;
    let mut text = String::new();
    for output_item in output {
        let Some(content_value) = output_item.get("content") else {
            continue;
        };
        let content = content_value.as_array().ok_or_else(|| {
            ModelDriverError::InvalidResponse(
                "the completed OpenAI response contained non-array content".to_owned(),
            )
        })?;
        for content_item in content {
            match content_item.get("type").and_then(Value::as_str) {
                Some("output_text") => {
                    let content_text = content_item
                        .get("text")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            ModelDriverError::InvalidResponse(
                                "completed OpenAI output text was not a string".to_owned(),
                            )
                        })?;
                    text.push_str(content_text);
                }
                Some("refusal") => {
                    let refusal = content_item
                        .get("refusal")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            ModelDriverError::InvalidResponse(
                                "completed OpenAI refusal was not a string".to_owned(),
                            )
                        })?;
                    text.push_str(refusal);
                }
                _ => {}
            }
        }
    }
    Ok((!text.is_empty()).then_some(text))
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::str::FromStr;
    use std::thread;

    use futures_util::StreamExt;
    use futures_util::stream;
    use reqwest::StatusCode;
    use serde_json::{Map, Value, json};
    use time::OffsetDateTime;

    use crate::conversation::{
        AssistantResponse, Conversation, ConversationEvent, ConversationEventId,
        ConversationEventKind, ConversationId, ModelCommunication, ModelEvent,
        ModelEventImportance, ModelId, ModelSource, ProviderId, UserContent,
    };
    use crate::model_driver::{ModelDriver, ModelDriverError};

    use super::{
        OpenAiModelDriver, ResponseByteStream, classify_response_error, model_communication,
        model_event_stream, semantic_input,
    };

    fn conversation_event(
        conversation_id: ConversationId,
        position: u64,
        kind: ConversationEventKind,
    ) -> ConversationEvent {
        ConversationEvent {
            conversation_id,
            position,
            id: ConversationEventId::new(),
            timestamp: OffsetDateTime::UNIX_EPOCH,
            schema_version: 7,
            kind,
        }
    }

    #[test]
    fn semantic_input_projects_canonical_events_and_ignores_extensions() {
        let conversation_id = ConversationId::new();
        let source = ModelSource::new(
            ProviderId::from_str("openai").expect("the provider identifier should be valid"),
            ModelId::from_str("gpt-5.6").expect("the model identifier should be valid"),
        );
        let extensions = Map::from_iter([(
            "unknown.extension".to_owned(),
            Value::String("ignored".to_owned()),
        )]);
        let conversation = Conversation::from_events(vec![
            conversation_event(
                conversation_id,
                0,
                ConversationEventKind::User {
                    content: vec![UserContent::Text("Hello".to_owned())],
                },
            ),
            conversation_event(
                conversation_id,
                1,
                ConversationEventKind::Model {
                    source: source.clone(),
                    event: ModelEvent::Communication(
                        ModelCommunication::new(
                            "Reasoning".to_owned(),
                            ModelEventImportance::Detailed,
                            "reasoning".to_owned(),
                            extensions.clone(),
                        )
                        .expect("the model communication should be valid"),
                    ),
                },
            ),
            conversation_event(
                conversation_id,
                2,
                ConversationEventKind::Model {
                    source,
                    event: ModelEvent::AssistantResponse(
                        AssistantResponse::new("Hello.".to_owned(), extensions)
                            .expect("the assistant response should be valid"),
                    ),
                },
            ),
        ])
        .expect("the conversation should be valid");

        assert_eq!(
            semantic_input(&conversation),
            json!([
                { "role": "user", "content": "Hello" },
                { "role": "assistant", "content": "Hello." }
            ])
        );
    }

    fn response_byte_stream(chunks: Vec<Vec<u8>>) -> ResponseByteStream {
        stream::iter(chunks.into_iter().map(Ok::<Vec<u8>, ModelDriverError>)).boxed()
    }

    fn one_byte_chunks(input: &str) -> Vec<Vec<u8>> {
        input.as_bytes().iter().map(|byte| vec![*byte]).collect()
    }

    async fn collect_events(input: &str) -> Vec<Result<ModelEvent, ModelDriverError>> {
        model_event_stream(response_byte_stream(vec![input.as_bytes().to_vec()]))
            .collect()
            .await
    }

    fn test_conversation() -> Conversation {
        let conversation_id = ConversationId::new();
        Conversation::from_events(vec![conversation_event(
            conversation_id,
            0,
            ConversationEventKind::User {
                content: vec![UserContent::Text("Hello".to_owned())],
            },
        )])
        .expect("the conversation should be valid")
    }

    fn source() -> ModelSource {
        ModelSource::new(
            ProviderId::from_str("openai").expect("the provider identifier should be valid"),
            ModelId::from_str("gpt-5.6").expect("the model identifier should be valid"),
        )
    }

    #[tokio::test]
    async fn invoke_returns_a_future_that_establishes_one_model_event_stream() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("the mock server should bind");
        let address = listener
            .local_addr()
            .expect("the mock server address should be available");
        let server = thread::spawn(move || {
            let (mut connection, _) = listener.accept().expect("the mock server should accept");
            read_request(&connection);
            let response_body = concat!(
                "data: {\"type\":\"response.reasoning_text.done\",\"text\":\"Reasoning\"}\n\n",
                "data: {\"type\":\"response.output_text.done\",\"text\":\"Answer\"}\n\n",
                "data: {\"type\":\"response.completed\",\"response\":{}}\n\n",
                "data: [DONE]\n\n"
            );
            write!(
                connection,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                response_body.len()
            )
            .expect("the mock response should write");
        });
        let driver = OpenAiModelDriver {
            http_client: reqwest::Client::new(),
            api_key: "test-key".to_owned(),
            responses_url: format!("http://{address}/responses"),
            source: source(),
        };

        let mut model_events = driver
            .invoke(&test_conversation())
            .await
            .expect("the invocation should establish its stream");
        let first_event = model_events
            .next()
            .await
            .expect("the stream should yield reasoning")
            .expect("the reasoning should be valid");
        let second_event = model_events
            .next()
            .await
            .expect("the stream should yield an answer")
            .expect("the answer should be valid");

        assert!(matches!(first_event, ModelEvent::Communication(_)));
        assert!(matches!(second_event, ModelEvent::AssistantResponse(_)));
        assert!(model_events.next().await.is_none());
        server.join().expect("the mock server should stop");
    }

    #[tokio::test]
    async fn an_early_http_failure_produces_no_model_stream() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("the mock server should bind");
        let address = listener
            .local_addr()
            .expect("the mock server address should be available");
        let server = thread::spawn(move || {
            let (mut connection, _) = listener.accept().expect("the mock server should accept");
            read_request(&connection);
            let body = "unauthorized";
            write!(
                connection,
                "HTTP/1.1 401 Unauthorized\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .expect("the mock response should write");
        });
        let driver = OpenAiModelDriver {
            http_client: reqwest::Client::new(),
            api_key: "test-key".to_owned(),
            responses_url: format!("http://{address}/responses"),
            source: source(),
        };

        let result = driver.invoke(&test_conversation()).await;

        assert!(matches!(result, Err(ModelDriverError::Authentication(_))));
        server.join().expect("the mock server should stop");
    }

    #[tokio::test]
    async fn arbitrary_byte_boundaries_crlf_multiline_data_and_event_fields_parse() {
        let input = concat!(
            "event: response.output_text.delta\r\n",
            "data: {\"delta\":\r\n",
            "data: \"Hello\"}\r\n\r\n",
            "event: response.output_text.done\r\n",
            "data: {}\r\n\r\n",
            "event: response.completed\r\n",
            "data: {\"response\":{}}\r\n\r\n",
            "data: [DONE]\r\n\r\n"
        );
        let mut model_events = model_event_stream(response_byte_stream(one_byte_chunks(input)));

        let model_event = model_events
            .next()
            .await
            .expect("the stream should yield an event")
            .expect("the event should be valid");

        assert_eq!(model_event.message(), "Hello");
        assert!(model_events.next().await.is_none());
    }

    #[tokio::test]
    async fn several_sse_events_in_one_chunk_yield_reasoning_before_the_answer() {
        let input = concat!(
            "data: {\"type\":\"response.reasoning_text.delta\",\"delta\":\"Detailed \"}\n\n",
            "data: {\"type\":\"response.reasoning_text.done\",\"text\":\"Detailed thought\"}\n\n",
            "data: {\"type\":\"response.reasoning_summary_text.done\",\"text\":\"Summary\"}\n\n",
            "data: {\"type\":\"response.output_text.done\",\"text\":\"Answer\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{}}\n\n"
        );

        let events = collect_events(input)
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .expect("the response stream should parse");

        assert_eq!(events.len(), 3);
        assert_eq!(events[0].message(), "Detailed thought");
        assert_eq!(events[0].importance(), ModelEventImportance::Detailed);
        assert_eq!(events[1].message(), "Summary");
        assert_eq!(events[1].importance(), ModelEventImportance::Interesting);
        assert_eq!(events[2].message(), "Answer");
        assert_eq!(events[2].importance(), ModelEventImportance::Important);
    }

    #[tokio::test]
    async fn a_late_stream_failure_follows_the_completed_model_event() {
        let input = concat!(
            "data: {\"type\":\"response.output_text.done\",\"text\":\"Hello\"}\n\n",
            "data: {\"type\":\"error\",\"message\":\"late failure\"}\n\n"
        );
        let mut model_events =
            model_event_stream(response_byte_stream(vec![input.as_bytes().to_vec()]));

        assert_eq!(
            model_events
                .next()
                .await
                .expect("the stream should yield an event")
                .expect("the completed event should be valid")
                .message(),
            "Hello"
        );
        assert!(matches!(
            model_events.next().await,
            Some(Err(ModelDriverError::Provider(_)))
        ));
        assert!(model_events.next().await.is_none());
    }

    #[tokio::test]
    async fn missing_response_completed_is_a_stream_error_even_after_done() {
        let input = concat!(
            "data: {\"type\":\"response.output_text.done\",\"text\":\"Hello\"}\n\n",
            "data: [DONE]\n\n"
        );
        let mut model_events =
            model_event_stream(response_byte_stream(vec![input.as_bytes().to_vec()]));

        assert!(matches!(
            model_events.next().await,
            Some(Ok(ModelEvent::AssistantResponse(_)))
        ));
        assert!(matches!(
            model_events.next().await,
            Some(Err(ModelDriverError::InvalidResponse(_)))
        ));
    }

    #[tokio::test]
    async fn response_completed_fallback_does_not_duplicate_a_done_event() {
        let input = concat!(
            "data: {\"type\":\"response.output_text.done\",\"text\":\"Answer\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"output\":[{\"content\":[{\"type\":\"output_text\",\"text\":\"Answer\"}]}]}}\n\n",
            "data: [DONE]\n\n"
        );

        let events = collect_events(input)
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .expect("the response stream should parse");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].message(), "Answer");
    }

    #[tokio::test]
    async fn distinct_indexed_output_completions_yield_distinct_semantic_events() {
        let input = concat!(
            "data: {\"type\":\"response.output_text.done\",\"output_index\":0,\"content_index\":0,\"text\":\"First\"}\n\n",
            "data: {\"type\":\"response.output_text.done\",\"output_index\":1,\"content_index\":0,\"text\":\"Second\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{}}\n\n"
        );

        let events = collect_events(input)
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .expect("the indexed output should parse");

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].message(), "First");
        assert_eq!(events[1].message(), "Second");
    }

    #[tokio::test]
    async fn duplicate_indexed_completion_is_a_stream_error_after_the_completed_event() {
        let input = concat!(
            "data: {\"type\":\"response.output_text.done\",\"output_index\":0,\"content_index\":0,\"text\":\"Answer\"}\n\n",
            "data: {\"type\":\"response.output_text.done\",\"output_index\":0,\"content_index\":0,\"text\":\"Answer\"}\n\n"
        );
        let mut events = model_event_stream(response_byte_stream(vec![input.as_bytes().to_vec()]));

        assert!(matches!(
            events.next().await,
            Some(Ok(ModelEvent::AssistantResponse(_)))
        ));
        assert!(matches!(
            events.next().await,
            Some(Err(ModelDriverError::InvalidResponse(_)))
        ));
    }

    #[tokio::test]
    async fn response_completed_supplies_final_object_fallback_at_end_of_stream() {
        let input = "data: {\"type\":\"response.completed\",\"response\":{\"output\":[{\"content\":[{\"type\":\"output_text\",\"text\":\"Fallback\"}]}]}}";

        let events = collect_events(input)
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .expect("the buffered final event should parse");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].message(), "Fallback");
    }

    #[tokio::test]
    async fn malformed_json_is_a_stream_error() {
        let mut events =
            model_event_stream(response_byte_stream(vec![b"data: not-json\n\n".to_vec()]));

        assert!(matches!(
            events.next().await,
            Some(Err(ModelDriverError::InvalidResponse(_)))
        ));
    }

    #[test]
    fn invalid_model_communication_maps_to_invalid_response() {
        let empty_message = model_communication(
            "   ".to_owned(),
            "reasoning",
            ModelEventImportance::Detailed,
        );
        let empty_subtype = model_communication(
            "reasoning".to_owned(),
            "   ",
            ModelEventImportance::Detailed,
        );

        assert!(matches!(
            empty_message,
            Err(ModelDriverError::InvalidResponse(_))
        ));
        assert!(matches!(
            empty_subtype,
            Err(ModelDriverError::InvalidResponse(_))
        ));
    }

    #[test]
    fn response_statuses_map_to_typed_driver_errors() {
        assert!(matches!(
            classify_response_error(StatusCode::UNAUTHORIZED, "unauthorized".to_owned()),
            ModelDriverError::Authentication(_)
        ));
        assert!(matches!(
            classify_response_error(StatusCode::TOO_MANY_REQUESTS, "slow down".to_owned()),
            ModelDriverError::RateLimited(_)
        ));
        assert!(matches!(
            classify_response_error(StatusCode::BAD_REQUEST, "bad request".to_owned()),
            ModelDriverError::InvalidRequest(_)
        ));
        assert!(matches!(
            classify_response_error(StatusCode::INTERNAL_SERVER_ERROR, "failed".to_owned()),
            ModelDriverError::Provider(_)
        ));
    }

    fn read_request(connection: &TcpStream) {
        let mut reader = BufReader::new(
            connection
                .try_clone()
                .expect("the request connection should clone"),
        );
        let mut content_length = None;
        loop {
            let mut header = String::new();
            reader
                .read_line(&mut header)
                .expect("the request header should read");
            if header == "\r\n" {
                break;
            }
            if let Some(length) = header.to_ascii_lowercase().strip_prefix("content-length:") {
                content_length = Some(
                    length
                        .trim()
                        .parse::<usize>()
                        .expect("the content length should be numeric"),
                );
            }
        }
        let mut body = vec![0; content_length.expect("the request should have a body")];
        reader
            .read_exact(&mut body)
            .expect("the request body should read");
    }
}
