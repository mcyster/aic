use std::error::Error;
use std::fmt::{Display, Formatter};
use std::io::{self, BufRead, BufReader};

use reqwest::StatusCode;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};

use crate::agent_run::ModelRequestSnapshot;
use crate::identifier::AgentRunEventId;

pub(crate) type OpenAiResult<T> = Result<T, Box<dyn Error>>;

pub(crate) struct OpenAiClient {
    http_client: Client,
    api_key: String,
    responses_url: String,
}

impl OpenAiClient {
    pub(crate) fn from_environment() -> OpenAiResult<Self> {
        let api_key = std::env::var("OPENAI_API_KEY")
            .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "OPENAI_API_KEY must be set"))?;
        let base_url = std::env::var("AIC_OPENAI_BASE_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1".to_owned());
        Ok(Self {
            http_client: Client::new(),
            api_key,
            responses_url: format!("{}/responses", base_url.trim_end_matches('/')),
        })
    }

    pub(crate) fn invoke(
        &self,
        request: &ModelRequestSnapshot,
        mut persist_provider_event: impl FnMut(String, Value) -> OpenAiResult<AgentRunEventId>,
    ) -> OpenAiResult<OpenAiOutput> {
        let mut request_body = Map::new();
        request_body.insert("model".to_owned(), Value::String(request.model.clone()));
        request_body.insert("input".to_owned(), request.input.clone());
        request_body.insert("stream".to_owned(), Value::Bool(true));
        request_body.insert("store".to_owned(), Value::Bool(true));
        if let Some(response_verbosity) = request.response_verbosity {
            request_body.insert(
                "text".to_owned(),
                json!({ "verbosity": response_verbosity.as_str() }),
            );
        }
        if let Some(previous_response_id) = &request.previous_response_id {
            request_body.insert(
                "previous_response_id".to_owned(),
                Value::String(previous_response_id.clone()),
            );
        }

        let response = self
            .http_client
            .post(&self.responses_url)
            .bearer_auth(&self.api_key)
            .json(&request_body)
            .send()?;
        let response_status = response.status();
        if !response_status.is_success() {
            let response_body = response.text()?;
            return Err(OpenAiResponseError {
                status: response_status,
                body: response_body,
            }
            .into());
        }

        parse_server_sent_events(BufReader::new(response), |event_type, payload| {
            persist_provider_event(event_type, payload)
        })
    }
}

pub(crate) struct OpenAiOutput {
    pub(crate) response_id: String,
    pub(crate) assistant_text: String,
    pub(crate) completion_event_id: AgentRunEventId,
}

fn parse_server_sent_events(
    mut reader: impl BufRead,
    mut persist_provider_event: impl FnMut(String, Value) -> OpenAiResult<AgentRunEventId>,
) -> OpenAiResult<OpenAiOutput> {
    let mut line = String::new();
    let mut event_name = None;
    let mut data_lines = Vec::new();
    let mut assistant_text = String::new();
    let mut completed_text = None;
    let mut response_id = None;
    let mut completion_event_id = None;

    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line)?;
        if bytes_read == 0 {
            process_event(
                &mut event_name,
                &mut data_lines,
                &mut assistant_text,
                &mut completed_text,
                &mut response_id,
                &mut completion_event_id,
                &mut persist_provider_event,
            )?;
            break;
        }

        let normalized_line = line.trim_end_matches(['\r', '\n']);
        if normalized_line.is_empty() {
            process_event(
                &mut event_name,
                &mut data_lines,
                &mut assistant_text,
                &mut completed_text,
                &mut response_id,
                &mut completion_event_id,
                &mut persist_provider_event,
            )?;
        } else if let Some(name) = normalized_line.strip_prefix("event:") {
            event_name = Some(name.trim_start().to_owned());
        } else if let Some(data) = normalized_line.strip_prefix("data:") {
            data_lines.push(data.trim_start().to_owned());
        }
    }

    let response_id = response_id.ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "response ID was not received")
    })?;
    let completion_event_id = completion_event_id.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "response.completed was not received",
        )
    })?;
    if assistant_text.is_empty()
        && let Some(text) = completed_text
    {
        assistant_text = text;
    }
    if assistant_text.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the completed response contained no assistant text",
        )
        .into());
    }

    Ok(OpenAiOutput {
        response_id,
        assistant_text,
        completion_event_id,
    })
}

#[allow(clippy::too_many_arguments)]
fn process_event(
    event_name: &mut Option<String>,
    data_lines: &mut Vec<String>,
    assistant_text: &mut String,
    completed_text: &mut Option<String>,
    response_id: &mut Option<String>,
    completion_event_id: &mut Option<AgentRunEventId>,
    persist_provider_event: &mut impl FnMut(String, Value) -> OpenAiResult<AgentRunEventId>,
) -> OpenAiResult<()> {
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

    let payload: Value = serde_json::from_str(&event_data)?;
    let event_type = payload
        .get("type")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| event_name.take())
        .unwrap_or_else(|| "unknown".to_owned());
    *event_name = None;
    let stored_event_id = persist_provider_event(event_type.clone(), payload.clone())?;

    match event_type.as_str() {
        "response.created" => {
            *response_id = response_identifier(&payload).map(str::to_owned);
        }
        "response.output_text.delta" => {
            if let Some(delta) = payload.get("delta").and_then(Value::as_str) {
                assistant_text.push_str(delta);
            }
        }
        "response.refusal.delta" => {
            if let Some(delta) = payload.get("delta").and_then(Value::as_str) {
                assistant_text.push_str(delta);
            }
        }
        "response.output_text.done" => {
            *completed_text = payload
                .get("text")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        "response.refusal.done" => {
            *completed_text = payload
                .get("refusal")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        "response.completed" => {
            if response_id.is_none() {
                *response_id = response_identifier(&payload).map(str::to_owned);
            }
            if completed_text.is_none() {
                *completed_text = completed_response_text(&payload);
            }
            *completion_event_id = Some(stored_event_id);
        }
        "error" | "response.failed" => {
            return Err(
                io::Error::other(format!("OpenAI stream emitted {event_type}: {payload}")).into(),
            );
        }
        _ => {}
    }
    Ok(())
}

fn response_identifier(payload: &Value) -> Option<&str> {
    payload
        .get("response")
        .and_then(|response| response.get("id"))
        .and_then(Value::as_str)
}

pub(crate) fn completed_response_text(payload: &Value) -> Option<String> {
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

#[derive(Debug)]
pub(crate) struct OpenAiResponseError {
    status: StatusCode,
    body: String,
}

impl OpenAiResponseError {
    pub(crate) fn permits_local_reconstruction(&self) -> bool {
        matches!(self.status, StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND)
    }
}

impl Display for OpenAiResponseError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "OpenAI Responses request failed with {}: {}",
            self.status, self.body
        )
    }
}

impl Error for OpenAiResponseError {}

pub(crate) fn semantic_input(messages: impl Iterator<Item = (String, String)>) -> Value {
    Value::Array(
        messages
            .map(|(role, content)| json!({ "role": role, "content": content }))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::parse_server_sent_events;

    #[test]
    fn streaming_events_are_persisted_before_projection() {
        let stream = concat!(
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hel\"}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"lo\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\"}}\n\n",
            "data: [DONE]\n\n"
        );
        let mut persisted_types = Vec::new();

        let output = parse_server_sent_events(Cursor::new(stream), |event_type, _| {
            persisted_types.push(event_type);
            Ok(crate::identifier::AgentRunEventId::new())
        })
        .expect("the response stream should parse");

        assert_eq!(output.response_id, "resp_1");
        assert_eq!(output.assistant_text, "Hello");
        assert_eq!(
            persisted_types,
            [
                "response.created",
                "response.output_text.delta",
                "response.output_text.delta",
                "response.completed"
            ]
        );
    }

    #[test]
    fn refusal_is_projected_as_assistant_text() {
        let stream = concat!(
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
            "data: {\"type\":\"response.refusal.delta\",\"delta\":\"I cannot\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\"}}\n\n"
        );

        let output = parse_server_sent_events(Cursor::new(stream), |_, _| {
            Ok(crate::identifier::AgentRunEventId::new())
        })
        .expect("the refusal stream should parse");

        assert_eq!(output.assistant_text, "I cannot");
    }
}
