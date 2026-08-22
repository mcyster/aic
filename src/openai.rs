use std::io::{BufRead, BufReader};
use std::str::FromStr;

use reqwest::StatusCode;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};

use crate::conversation::{
    AssistantResponse, Conversation, ConversationEvent, ModelCommunication, ModelEvent,
    ModelEventImportance, ModelId, ModelSource, ProviderId, UserContent,
};
use crate::model_driver::{ModelDriver, ModelDriverError};

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

    fn invoke(&self, conversation: &Conversation) -> Result<Vec<ModelEvent>, ModelDriverError> {
        let mut request_body = Map::new();
        request_body.insert(
            "model".to_owned(),
            Value::String(self.source.model().as_str().to_owned()),
        );
        request_body.insert("input".to_owned(), semantic_input(conversation));
        request_body.insert("reasoning".to_owned(), json!({ "summary": "auto" }));
        request_body.insert("stream".to_owned(), Value::Bool(true));
        request_body.insert("store".to_owned(), Value::Bool(true));

        let response = self
            .http_client
            .post(&self.responses_url)
            .bearer_auth(&self.api_key)
            .json(&request_body)
            .send()
            .map_err(|error| ModelDriverError::Transport(error.to_string()))?;
        let response_status = response.status();
        if !response_status.is_success() {
            let response_body = response
                .text()
                .map_err(|error| ModelDriverError::Transport(error.to_string()))?;
            return Err(classify_response_error(response_status, response_body));
        }

        parse_server_sent_events(BufReader::new(response))
    }
}

fn semantic_input(conversation: &Conversation) -> Value {
    Value::Array(
        conversation
            .events()
            .iter()
            .filter_map(|stored_event| match &stored_event.event {
                ConversationEvent::User { content } => {
                    let text = content
                        .iter()
                        .map(|content| match content {
                            UserContent::Text(text) => text.as_str(),
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    Some(json!({ "role": "user", "content": text }))
                }
                ConversationEvent::Model {
                    event: ModelEvent::AssistantResponse(response),
                    ..
                } => Some(json!({ "role": "assistant", "content": response.message() })),
                ConversationEvent::Model { .. } => None,
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

fn parse_server_sent_events(mut reader: impl BufRead) -> Result<Vec<ModelEvent>, ModelDriverError> {
    let mut line = String::new();
    let mut event_name = None;
    let mut data_lines = Vec::new();
    let mut response_state = ResponseState::default();

    loop {
        line.clear();
        let bytes_read = reader
            .read_line(&mut line)
            .map_err(|error| ModelDriverError::Transport(error.to_string()))?;
        if bytes_read == 0 {
            process_event(&mut event_name, &mut data_lines, &mut response_state)?;
            break;
        }

        let normalized_line = line.trim_end_matches(['\r', '\n']);
        if normalized_line.is_empty() {
            process_event(&mut event_name, &mut data_lines, &mut response_state)?;
        } else if let Some(name) = normalized_line.strip_prefix("event:") {
            event_name = Some(name.trim_start().to_owned());
        } else if let Some(data) = normalized_line.strip_prefix("data:") {
            data_lines.push(data.trim_start().to_owned());
        }
    }

    if !response_state.completed {
        return Err(ModelDriverError::InvalidResponse(
            "response.completed was not received".to_owned(),
        ));
    }
    Ok(response_state.events)
}

#[derive(Default)]
struct ResponseState {
    assistant_text: String,
    completed_assistant_text: Option<String>,
    reasoning_text: String,
    completed_reasoning_text: Option<String>,
    reasoning_summary: String,
    completed_reasoning_summary: Option<String>,
    events: Vec<ModelEvent>,
    completed: bool,
}

fn process_event(
    event_name: &mut Option<String>,
    data_lines: &mut Vec<String>,
    response_state: &mut ResponseState,
) -> Result<(), ModelDriverError> {
    if data_lines.is_empty() {
        *event_name = None;
        return Ok(());
    }
    let event_data = data_lines.join("\n");
    data_lines.clear();
    if event_data == "[DONE]" {
        *event_name = None;
        return Ok(());
    }

    let payload: Value = serde_json::from_str(&event_data)
        .map_err(|error| ModelDriverError::InvalidResponse(error.to_string()))?;
    let event_type = payload
        .get("type")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| event_name.take())
        .unwrap_or_else(|| "unknown".to_owned());
    *event_name = None;

    match event_type.as_str() {
        "response.output_text.delta" | "response.refusal.delta" => {
            append_delta(&payload, &mut response_state.assistant_text);
        }
        "response.output_text.done" => {
            response_state.completed_assistant_text = text_field(&payload, "text");
        }
        "response.refusal.done" => {
            response_state.completed_assistant_text = text_field(&payload, "refusal");
        }
        "response.reasoning_text.delta" => {
            append_delta(&payload, &mut response_state.reasoning_text);
        }
        "response.reasoning_text.done" => {
            response_state.completed_reasoning_text = text_field(&payload, "text");
        }
        "response.reasoning_summary_text.delta" => {
            append_delta(&payload, &mut response_state.reasoning_summary);
        }
        "response.reasoning_summary_text.done" => {
            response_state.completed_reasoning_summary = text_field(&payload, "text");
        }
        "response.completed" => complete_response(response_state, &payload)?,
        "error" | "response.failed" => {
            return Err(ModelDriverError::Provider(format!(
                "OpenAI stream emitted {event_type}: {payload}"
            )));
        }
        _ => {}
    }
    Ok(())
}

fn complete_response(
    response_state: &mut ResponseState,
    payload: &Value,
) -> Result<(), ModelDriverError> {
    if response_state.completed {
        return Err(ModelDriverError::InvalidResponse(
            "response.completed was received more than once".to_owned(),
        ));
    }

    if let Some(reasoning_text) = preferred_text(
        &response_state.reasoning_text,
        &response_state.completed_reasoning_text,
    ) {
        response_state.events.push(model_communication(
            reasoning_text,
            "reasoning",
            ModelEventImportance::Detailed,
        ));
    }
    if let Some(reasoning_summary) = preferred_text(
        &response_state.reasoning_summary,
        &response_state.completed_reasoning_summary,
    ) {
        response_state.events.push(model_communication(
            reasoning_summary,
            "reasoning_summary",
            ModelEventImportance::Interesting,
        ));
    }

    let assistant_text = preferred_text(
        &response_state.assistant_text,
        &response_state.completed_assistant_text,
    )
    .or_else(|| completed_response_text(payload))
    .ok_or_else(|| {
        ModelDriverError::InvalidResponse(
            "the completed response contained no model message".to_owned(),
        )
    })?;
    response_state
        .events
        .push(ModelEvent::AssistantResponse(AssistantResponse::new(
            assistant_text,
            Map::new(),
        )));
    response_state.completed = true;
    Ok(())
}

fn model_communication(
    message: String,
    subtype: &str,
    importance: ModelEventImportance,
) -> ModelEvent {
    ModelEvent::Communication(ModelCommunication::new(
        message,
        importance,
        subtype.to_owned(),
        Map::new(),
    ))
}

fn append_delta(payload: &Value, text: &mut String) {
    if let Some(delta) = payload.get("delta").and_then(Value::as_str) {
        text.push_str(delta);
    }
}

fn text_field(payload: &Value, field: &str) -> Option<String> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn preferred_text(streamed_text: &str, completed_text: &Option<String>) -> Option<String> {
    if streamed_text.is_empty() {
        completed_text.clone()
    } else {
        Some(streamed_text.to_owned())
    }
}

fn completed_response_text(payload: &Value) -> Option<String> {
    let output = payload.get("response")?.get("output")?.as_array()?;
    let mut text = String::new();
    for output_item in output {
        let Some(content) = output_item.get("content").and_then(Value::as_array) else {
            continue;
        };
        for content_item in content {
            match content_item.get("type").and_then(Value::as_str) {
                Some("output_text") => {
                    if let Some(content_text) = content_item.get("text").and_then(Value::as_str) {
                        text.push_str(content_text);
                    }
                }
                Some("refusal") => {
                    if let Some(refusal) = content_item.get("refusal").and_then(Value::as_str) {
                        text.push_str(refusal);
                    }
                }
                _ => {}
            }
        }
    }
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use reqwest::StatusCode;
    use serde_json::Map;

    use crate::conversation::{ModelCommunication, ModelEvent, ModelEventImportance};
    use crate::model_driver::ModelDriverError;

    use super::{classify_response_error, parse_server_sent_events};

    #[test]
    fn streaming_response_returns_assistant_response() {
        let stream = concat!(
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hel\"}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"lo\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\"}}\n\n",
            "data: [DONE]\n\n"
        );

        let events = parse_server_sent_events(Cursor::new(stream))
            .expect("the response stream should parse");

        let ModelEvent::AssistantResponse(response) = &events[0] else {
            panic!("the returned event should be an assistant response");
        };
        assert_eq!(response.message(), "Hello");
        assert_eq!(events[0].importance(), ModelEventImportance::Important);
    }

    #[test]
    fn exposed_reasoning_is_aggregated_by_importance() {
        let stream = concat!(
            "data: {\"type\":\"response.reasoning_text.delta\",\"delta\":\"Detailed \"}\n\n",
            "data: {\"type\":\"response.reasoning_text.delta\",\"delta\":\"thought\"}\n\n",
            "data: {\"type\":\"response.reasoning_summary_text.delta\",\"delta\":\"Summary\"}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Answer\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{}}\n\n"
        );

        let events = parse_server_sent_events(Cursor::new(stream))
            .expect("the response stream should parse");

        assert_eq!(events.len(), 3);
        assert_eq!(
            events
                .iter()
                .map(ModelEvent::importance)
                .collect::<Vec<_>>(),
            [
                ModelEventImportance::Detailed,
                ModelEventImportance::Interesting,
                ModelEventImportance::Important
            ]
        );
        assert_eq!(
            events[0],
            ModelEvent::Communication(ModelCommunication::new(
                "Detailed thought".to_owned(),
                ModelEventImportance::Detailed,
                "reasoning".to_owned(),
                Map::new(),
            ))
        );
        assert_eq!(
            events[1],
            ModelEvent::Communication(ModelCommunication::new(
                "Summary".to_owned(),
                ModelEventImportance::Interesting,
                "reasoning_summary".to_owned(),
                Map::new(),
            ))
        );
        let ModelEvent::AssistantResponse(response) = &events[2] else {
            panic!("the final event should be an assistant response");
        };
        assert_eq!(response.message(), "Answer");
    }

    #[test]
    fn a_late_stream_failure_discards_model_events() {
        let stream = concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hello\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{}}\n\n",
            "data: {\"type\":\"error\",\"message\":\"late failure\"}\n\n"
        );

        let result = parse_server_sent_events(Cursor::new(stream));

        assert!(matches!(result, Err(ModelDriverError::Provider(_))));
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
}
