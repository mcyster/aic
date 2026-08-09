use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde::de::DeserializeOwned;
use uuid::Uuid;

use crate::agent_run::{AgentRun, AgentRunEvent, StoredAgentRunEvent};
use crate::conversation::{Conversation, ConversationEvent, StoredConversationEvent};
use crate::identifier::{AgentRunEventId, AgentRunId, ConversationEventId, ConversationId};

const SCHEMA_VERSION: u32 = 1;

pub(crate) struct EventStore {
    root_directory: PathBuf,
}

impl EventStore {
    pub(crate) fn from_environment() -> io::Result<Self> {
        let root_directory = if let Some(configured_directory) = std::env::var_os("AIC_DATA_DIR") {
            PathBuf::from(configured_directory)
        } else if let Some(data_home) = std::env::var_os("XDG_DATA_HOME") {
            PathBuf::from(data_home).join("aic")
        } else if let Some(home_directory) = std::env::var_os("HOME") {
            PathBuf::from(home_directory).join(".local/share/aic")
        } else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "AIC_DATA_DIR, XDG_DATA_HOME, or HOME must be set",
            ));
        };

        Self::new(root_directory)
    }

    pub(crate) fn new(root_directory: PathBuf) -> io::Result<Self> {
        create_private_directory(&root_directory)?;
        create_private_directory(&root_directory.join("conversations"))?;
        create_private_directory(&root_directory.join("agent-runs"))?;
        Ok(Self { root_directory })
    }

    pub(crate) fn create_conversation(&self) -> io::Result<Conversation> {
        let conversation = Conversation {
            id: ConversationId::new(),
            created_at_milliseconds: current_timestamp_milliseconds()?,
        };
        let conversation_directory = self.conversation_directory(conversation.id);
        create_private_directory(&conversation_directory)?;
        create_private_directory(&conversation_directory.join("events"))?;
        write_json_atomically(
            &conversation_directory.join("conversation.json"),
            &conversation,
        )?;
        Ok(conversation)
    }

    pub(crate) fn load_conversation(
        &self,
        conversation_id: ConversationId,
    ) -> io::Result<Conversation> {
        read_json(
            &self
                .conversation_directory(conversation_id)
                .join("conversation.json"),
        )
    }

    pub(crate) fn append_conversation_event(
        &self,
        conversation_id: ConversationId,
        event: ConversationEvent,
    ) -> io::Result<StoredConversationEvent> {
        let existing_events = self.load_conversation_events(conversation_id)?;
        if let Some((event_kind, projection_identity)) = event.projection_identity()
            && let Some(existing_event) = existing_events.iter().find(|stored_event| {
                stored_event.event.projection_identity().is_some_and(
                    |(existing_kind, existing_identity)| {
                        existing_kind == event_kind && existing_identity == projection_identity
                    },
                )
            })
        {
            return Ok(existing_event.clone());
        }

        let stored_event = StoredConversationEvent {
            position: next_position(existing_events.last().map(|event| event.position))?,
            id: ConversationEventId::new(),
            timestamp_milliseconds: current_timestamp_milliseconds()?,
            schema_version: SCHEMA_VERSION,
            event,
        };
        let events_directory = self.conversation_directory(conversation_id).join("events");
        write_json_atomically(
            &event_path(
                &events_directory,
                stored_event.position,
                &stored_event.id.storage_key(),
            ),
            &stored_event,
        )?;
        Ok(stored_event)
    }

    pub(crate) fn load_conversation_events(
        &self,
        conversation_id: ConversationId,
    ) -> io::Result<Vec<StoredConversationEvent>> {
        let mut events =
            read_json_directory(&self.conversation_directory(conversation_id).join("events"))?;
        events.sort_by_key(|event: &StoredConversationEvent| event.position);
        validate_positions(events.iter().map(|event| event.position))?;
        Ok(events)
    }

    pub(crate) fn create_agent_run(&self, conversation_id: ConversationId) -> io::Result<AgentRun> {
        let agent_run = AgentRun {
            id: AgentRunId::new(),
            conversation_id,
            created_at_milliseconds: current_timestamp_milliseconds()?,
        };
        let agent_run_directory = self.agent_run_directory(agent_run.id);
        create_private_directory(&agent_run_directory)?;
        create_private_directory(&agent_run_directory.join("events"))?;
        write_json_atomically(&agent_run_directory.join("agent-run.json"), &agent_run)?;
        Ok(agent_run)
    }

    pub(crate) fn append_agent_run_event(
        &self,
        agent_run_id: AgentRunId,
        event: AgentRunEvent,
    ) -> io::Result<StoredAgentRunEvent> {
        let existing_events = self.load_agent_run_events(agent_run_id)?;
        let stored_event = StoredAgentRunEvent {
            position: next_position(existing_events.last().map(|event| event.position))?,
            id: AgentRunEventId::new(),
            timestamp_milliseconds: current_timestamp_milliseconds()?,
            schema_version: SCHEMA_VERSION,
            event,
        };
        let events_directory = self.agent_run_directory(agent_run_id).join("events");
        write_json_atomically(
            &event_path(
                &events_directory,
                stored_event.position,
                &stored_event.id.storage_key(),
            ),
            &stored_event,
        )?;
        Ok(stored_event)
    }

    pub(crate) fn load_agent_run_events(
        &self,
        agent_run_id: AgentRunId,
    ) -> io::Result<Vec<StoredAgentRunEvent>> {
        let mut events =
            read_json_directory(&self.agent_run_directory(agent_run_id).join("events"))?;
        events.sort_by_key(|event: &StoredAgentRunEvent| event.position);
        validate_positions(events.iter().map(|event| event.position))?;
        Ok(events)
    }

    pub(crate) fn load_agent_runs(
        &self,
        conversation_id: ConversationId,
    ) -> io::Result<Vec<AgentRun>> {
        let mut agent_runs = Vec::new();
        for directory_entry in fs::read_dir(self.root_directory.join("agent-runs"))? {
            let directory_entry = directory_entry?;
            if !directory_entry.file_type()?.is_dir() {
                continue;
            }
            let metadata_path = directory_entry.path().join("agent-run.json");
            if !metadata_path.exists() {
                continue;
            }
            let agent_run: AgentRun = read_json(&metadata_path)?;
            if agent_run.conversation_id == conversation_id {
                agent_runs.push(agent_run);
            }
        }
        agent_runs.sort_by_key(|agent_run| {
            (
                agent_run.created_at_milliseconds,
                agent_run.id.storage_key(),
            )
        });
        Ok(agent_runs)
    }

    fn conversation_directory(&self, conversation_id: ConversationId) -> PathBuf {
        self.root_directory
            .join("conversations")
            .join(conversation_id.storage_key())
    }

    fn agent_run_directory(&self, agent_run_id: AgentRunId) -> PathBuf {
        self.root_directory
            .join("agent-runs")
            .join(agent_run_id.storage_key())
    }
}

fn create_private_directory(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

fn current_timestamp_milliseconds() -> io::Result<u64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?;
    u64::try_from(duration.as_millis()).map_err(io::Error::other)
}

fn next_position(previous_position: Option<u64>) -> io::Result<u64> {
    match previous_position {
        Some(position) => position
            .checked_add(1)
            .ok_or_else(|| io::Error::other("event position overflow")),
        None => Ok(0),
    }
}

fn event_path(directory: &Path, position: u64, identifier: &str) -> PathBuf {
    directory.join(format!("{position:020}-{identifier}.json"))
}

fn write_json_atomically<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let parent_directory = path
        .parent()
        .ok_or_else(|| io::Error::other("persisted file has no parent directory"))?;
    let temporary_path = parent_directory.join(format!(".tmp-{}", Uuid::now_v7().simple()));
    let mut temporary_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary_path)?;
    serde_json::to_writer(&mut temporary_file, value).map_err(io::Error::other)?;
    temporary_file.write_all(b"\n")?;
    temporary_file.sync_all()?;
    fs::rename(&temporary_path, path)?;
    File::open(parent_directory)?.sync_all()
}

fn read_json<T: DeserializeOwned>(path: &Path) -> io::Result<T> {
    let file = File::open(path)?;
    serde_json::from_reader(BufReader::new(file)).map_err(io::Error::other)
}

fn read_json_directory<T: DeserializeOwned>(directory: &Path) -> io::Result<Vec<T>> {
    let mut values = Vec::new();
    for directory_entry in fs::read_dir(directory)? {
        let directory_entry = directory_entry?;
        let path = directory_entry.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            values.push(read_json(&path)?);
        }
    }
    Ok(values)
}

fn validate_positions(positions: impl Iterator<Item = u64>) -> io::Result<()> {
    for (expected_position, position) in positions.enumerate() {
        let expected_position = u64::try_from(expected_position).map_err(io::Error::other)?;
        if position != expected_position {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("expected event position {expected_position}, found {position}"),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::EventStore;
    use crate::agent_run::AgentRunEvent;
    use crate::conversation::{ConversationEvent, ProjectionIdentity};

    fn temporary_store() -> EventStore {
        let directory = std::env::temp_dir().join(format!("aic-test-{}", uuid::Uuid::now_v7()));
        EventStore::new(directory).expect("the event store should be created")
    }

    #[test]
    fn events_receive_monotonic_positions() {
        let store = temporary_store();
        let conversation = store
            .create_conversation()
            .expect("the conversation should be created");

        store
            .append_conversation_event(
                conversation.id,
                ConversationEvent::User {
                    text: "first".to_owned(),
                },
            )
            .expect("the first event should be persisted");
        store
            .append_conversation_event(
                conversation.id,
                ConversationEvent::User {
                    text: "second".to_owned(),
                },
            )
            .expect("the second event should be persisted");

        let events = store
            .load_conversation_events(conversation.id)
            .expect("the events should load");
        assert_eq!(events[0].position, 0);
        assert_eq!(events[1].position, 1);
    }

    #[test]
    fn semantic_projection_is_idempotent() {
        let store = temporary_store();
        let conversation = store
            .create_conversation()
            .expect("the conversation should be created");
        let agent_run = store
            .create_agent_run(conversation.id)
            .expect("the agent run should be created");
        let source_event = store
            .append_agent_run_event(
                agent_run.id,
                AgentRunEvent::RunCompleted {
                    response_id: "resp_test".to_owned(),
                },
            )
            .expect("the source event should be persisted");
        let assistant_event = ConversationEvent::Assistant {
            text: "hello".to_owned(),
            projection: ProjectionIdentity {
                source_run_id: agent_run.id,
                source_run_event_id: source_event.id,
                output_index: 0,
            },
        };

        let first = store
            .append_conversation_event(conversation.id, assistant_event.clone())
            .expect("the first projection should be persisted");
        let second = store
            .append_conversation_event(conversation.id, assistant_event)
            .expect("the repeated projection should resolve");

        assert_eq!(first.id, second.id);
        assert_eq!(
            store
                .load_conversation_events(conversation.id)
                .expect("the events should load")
                .len(),
            1
        );
    }
}
