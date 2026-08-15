use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use serde_json::Value;

fn tog_command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_tog"))
}

fn temporary_data_directory() -> PathBuf {
    std::env::temp_dir().join(format!("tog-command-test-{}", uuid::Uuid::now_v7()))
}

struct MockOpenAiServer {
    base_url: String,
    requests: Arc<Mutex<Vec<Value>>>,
    server_thread: JoinHandle<()>,
}

enum MockResponse {
    Success {
        response_id: &'static str,
        assistant_text: &'static str,
    },
    Failure {
        status: &'static str,
    },
}

impl MockOpenAiServer {
    fn start(responses: Vec<MockResponse>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("the mock server should bind");
        let address = listener
            .local_addr()
            .expect("the mock server address should be available");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let thread_requests = Arc::clone(&requests);
        let server_thread = thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().expect("the mock server should accept");
                let request = read_request(&stream);
                thread_requests
                    .lock()
                    .expect("the request list should lock")
                    .push(request);
                write_response(&mut stream, response);
            }
        });
        Self {
            base_url: format!("http://{address}/v1"),
            requests,
            server_thread,
        }
    }

    fn finish(self) -> Vec<Value> {
        self.server_thread
            .join()
            .expect("the mock server should stop cleanly");
        Arc::try_unwrap(self.requests)
            .expect("the request list should have one owner")
            .into_inner()
            .expect("the request list should unlock")
    }
}

fn read_request(stream: &TcpStream) -> Value {
    let mut reader = BufReader::new(stream.try_clone().expect("the request stream should clone"));
    let mut content_length = None;
    loop {
        let mut header_line = String::new();
        reader
            .read_line(&mut header_line)
            .expect("the request header should read");
        if header_line == "\r\n" {
            break;
        }
        if let Some(length) = header_line
            .to_ascii_lowercase()
            .strip_prefix("content-length:")
        {
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
    serde_json::from_slice(&body).expect("the request body should be JSON")
}

fn write_response(stream: &mut TcpStream, response: MockResponse) {
    let (status, content_type, response_body) = match response {
        MockResponse::Success {
            response_id,
            assistant_text,
        } => (
            "200 OK",
            "text/event-stream",
            format!(
                concat!(
                    "data: {{\"type\":\"response.created\",\"response\":{{\"id\":\"{}\"}}}}\n\n",
                    "data: {{\"type\":\"response.output_text.delta\",\"delta\":\"{}\"}}\n\n",
                    "data: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"{}\"}}}}\n\n",
                    "data: [DONE]\n\n"
                ),
                response_id, assistant_text, response_id
            ),
        ),
        MockResponse::Failure { status } => (
            status,
            "application/json",
            "{\"error\":{\"message\":\"request rejected\"}}".to_owned(),
        ),
    };
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
        response_body.len(),
    )
    .expect("the mock response should write");
}

fn configured_command(server: &MockOpenAiServer, data_directory: &PathBuf) -> Command {
    let mut command = tog_command();
    command
        .env("OPENAI_API_KEY", "test-key")
        .env("TOG_OPENAI_BASE_URL", &server.base_url)
        .env("TOG_DATA_DIR", data_directory);
    command
}

fn reported_conversation_id(standard_error: &[u8]) -> String {
    String::from_utf8(standard_error.to_vec())
        .expect("standard error should be UTF-8")
        .lines()
        .find_map(|line| line.strip_prefix("#> conversation "))
        .expect("standard error should identify the conversation")
        .to_owned()
}

#[test]
fn turn_persists_events_and_prints_semantic_output() {
    let server = MockOpenAiServer::start(vec![MockResponse::Success {
        response_id: "resp_first",
        assistant_text: "Hello",
    }]);
    let data_directory = temporary_data_directory();

    let command_output = configured_command(&server, &data_directory)
        .args(["turn", "Say hello"])
        .output()
        .expect("tog should run");

    assert!(command_output.status.success());
    assert_eq!(
        String::from_utf8(command_output.stdout).expect("standard output should be UTF-8"),
        "Hello\n"
    );
    let standard_error =
        String::from_utf8(command_output.stderr.clone()).expect("standard error should be UTF-8");
    assert!(reported_conversation_id(&command_output.stderr).starts_with("conversation_"));
    assert!(standard_error.contains("## waiting for OpenAI model gpt-5.6\n"));
    assert!(standard_error.contains("## receiving OpenAI response (1 provider event)\n"));
    let requests = server.finish();
    assert_eq!(requests[0]["model"], "gpt-5.6");
    assert_eq!(requests[0]["input"][0]["content"], "Say hello");
    assert_eq!(requests[0]["stream"], true);
    assert_eq!(requests[0]["text"]["verbosity"], "low");
}

#[test]
fn turn_sends_selected_response_verbosity() {
    let server = MockOpenAiServer::start(vec![MockResponse::Success {
        response_id: "resp_verbose",
        assistant_text: "Detailed answer",
    }]);
    let data_directory = temporary_data_directory();

    let command_output = configured_command(&server, &data_directory)
        .args(["turn", "--verbosity", "high", "Explain ownership"])
        .output()
        .expect("tog should run");

    assert!(command_output.status.success());
    let requests = server.finish();
    assert_eq!(requests[0]["text"]["verbosity"], "high");
}

#[test]
fn subsequent_turn_continues_from_the_previous_response() {
    let server = MockOpenAiServer::start(vec![
        MockResponse::Success {
            response_id: "resp_first",
            assistant_text: "First answer",
        },
        MockResponse::Success {
            response_id: "resp_second",
            assistant_text: "Second answer",
        },
    ]);
    let data_directory = temporary_data_directory();
    let first_output = configured_command(&server, &data_directory)
        .args(["turn", "First question"])
        .output()
        .expect("the first turn should run");
    assert!(first_output.status.success());
    let conversation_id = reported_conversation_id(&first_output.stderr);

    let second_output = configured_command(&server, &data_directory)
        .args([
            "turn",
            "--conversation",
            &conversation_id,
            "Second question",
        ])
        .output()
        .expect("the second turn should run");

    assert!(second_output.status.success());
    assert_eq!(
        String::from_utf8(second_output.stdout).expect("standard output should be UTF-8"),
        "Second answer\n"
    );
    let requests = server.finish();
    assert_eq!(requests[1]["previous_response_id"], "resp_first");
    assert_eq!(requests[1]["input"][0]["content"], "Second question");
}

#[test]
fn rejected_continuation_falls_back_to_local_semantic_history() {
    let server = MockOpenAiServer::start(vec![
        MockResponse::Success {
            response_id: "resp_first",
            assistant_text: "First answer",
        },
        MockResponse::Failure {
            status: "404 Not Found",
        },
        MockResponse::Success {
            response_id: "resp_reconstructed",
            assistant_text: "Reconstructed answer",
        },
    ]);
    let data_directory = temporary_data_directory();
    let first_output = configured_command(&server, &data_directory)
        .args(["turn", "First question"])
        .output()
        .expect("the first turn should run");
    let conversation_id = reported_conversation_id(&first_output.stderr);

    let second_output = configured_command(&server, &data_directory)
        .args([
            "turn",
            "--conversation",
            &conversation_id,
            "Second question",
        ])
        .output()
        .expect("the second turn should run");

    assert!(second_output.status.success());
    assert_eq!(
        String::from_utf8(second_output.stdout).expect("standard output should be UTF-8"),
        "Reconstructed answer\n"
    );
    let requests = server.finish();
    assert_eq!(requests[1]["previous_response_id"], "resp_first");
    assert!(requests[2].get("previous_response_id").is_none());
    assert_eq!(requests[2]["input"].as_array().map(Vec::len), Some(3));
}

#[test]
fn failed_user_turn_is_included_in_the_next_local_reconstruction() {
    let server = MockOpenAiServer::start(vec![
        MockResponse::Success {
            response_id: "resp_first",
            assistant_text: "First answer",
        },
        MockResponse::Failure {
            status: "500 Internal Server Error",
        },
        MockResponse::Success {
            response_id: "resp_after_failure",
            assistant_text: "Recovered answer",
        },
    ]);
    let data_directory = temporary_data_directory();
    let first_output = configured_command(&server, &data_directory)
        .args(["turn", "First question"])
        .output()
        .expect("the first turn should run");
    let conversation_id = reported_conversation_id(&first_output.stderr);
    let failed_output = configured_command(&server, &data_directory)
        .args([
            "turn",
            "--conversation",
            &conversation_id,
            "Failed question",
        ])
        .output()
        .expect("the failed turn should run");
    assert!(!failed_output.status.success());
    assert_eq!(
        reported_conversation_id(&failed_output.stderr),
        conversation_id
    );

    let recovered_output = configured_command(&server, &data_directory)
        .args([
            "turn",
            "--conversation",
            &conversation_id,
            "Recovery question",
        ])
        .output()
        .expect("the recovery turn should run");

    assert!(recovered_output.status.success());
    let requests = server.finish();
    assert!(requests[2].get("previous_response_id").is_none());
    assert_eq!(requests[2]["input"].as_array().map(Vec::len), Some(4));
    assert_eq!(requests[2]["input"][2]["content"], "Failed question");
}

#[test]
fn turn_rejects_a_missing_user_prompt() {
    let command_output = tog_command().arg("turn").output().expect("tog should run");

    assert!(!command_output.status.success());
    assert!(
        String::from_utf8(command_output.stderr)
            .expect("standard error should be UTF-8")
            .contains("required")
    );
}

#[test]
fn help_lists_the_turn_command() {
    let command_output = tog_command()
        .arg("--help")
        .output()
        .expect("tog should run");

    assert!(command_output.status.success());
    assert!(
        String::from_utf8(command_output.stdout)
            .expect("standard output should be UTF-8")
            .contains("turn")
    );
}
