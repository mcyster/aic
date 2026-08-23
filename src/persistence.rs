use std::error::Error;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::conversation::{
    Conversation, ConversationEvent, ConversationEventId, ConversationEventKind, ConversationId,
};

const SCHEMA_VERSION: u32 = 9;

pub(crate) struct EventStore {
    root_directory: PathBuf,
}

impl EventStore {
    pub(crate) fn from_environment() -> io::Result<Self> {
        let root_directory = if let Some(configured_directory) = std::env::var_os("TOG_DATA_DIR") {
            PathBuf::from(configured_directory)
        } else if let Some(data_home) = std::env::var_os("XDG_DATA_HOME") {
            PathBuf::from(data_home).join("tog")
        } else if let Some(home_directory) = std::env::var_os("HOME") {
            PathBuf::from(home_directory).join(".local/share/tog")
        } else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "TOG_DATA_DIR, XDG_DATA_HOME, or HOME must be set",
            ));
        };

        Self::new(root_directory)
    }

    pub(crate) fn new(root_directory: PathBuf) -> io::Result<Self> {
        create_private_directory(&root_directory)?;
        create_private_directory(&root_directory.join("conversations"))?;
        Ok(Self { root_directory })
    }

    pub(crate) fn load_conversation(
        &self,
        conversation_id: ConversationId,
    ) -> io::Result<Conversation> {
        let events = self.load_conversation_events(conversation_id)?;
        let conversation = Conversation::from_events(events).map_err(invalid_conversation_data)?;
        if conversation.id() != conversation_id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("loaded {}, expected {conversation_id}", conversation.id()),
            ));
        }
        Ok(conversation)
    }

    pub(crate) fn append_conversation_event(
        &self,
        conversation_id: ConversationId,
        kind: ConversationEventKind,
    ) -> io::Result<ConversationEvent> {
        let conversation_directory = self.conversation_directory(conversation_id);
        create_private_directory(&conversation_directory)?;
        let events_directory = conversation_directory.join("events");
        create_private_directory(&events_directory)?;
        let existing_events = self.load_conversation_events(conversation_id)?;
        let previous_position = if existing_events.is_empty() {
            None
        } else {
            let conversation =
                Conversation::from_events(existing_events).map_err(invalid_conversation_data)?;
            if conversation.id() != conversation_id {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("loaded {}, expected {conversation_id}", conversation.id()),
                ));
            }
            conversation.events().last().map(|event| event.position)
        };
        let conversation_event = ConversationEvent {
            conversation_id,
            position: next_position(previous_position)?,
            id: ConversationEventId::new(),
            timestamp: OffsetDateTime::now_utc(),
            schema_version: SCHEMA_VERSION,
            kind,
        };
        write_json_atomically(
            &event_path(
                &events_directory,
                conversation_event.position,
                &conversation_event.id.storage_key(),
            ),
            &conversation_event,
        )?;
        Ok(conversation_event)
    }

    fn load_conversation_events(
        &self,
        conversation_id: ConversationId,
    ) -> io::Result<Vec<ConversationEvent>> {
        let mut events =
            read_json_directory(&self.conversation_directory(conversation_id).join("events"))?;
        events.sort_by_key(|event: &ConversationEvent| event.position);
        Ok(events)
    }

    fn conversation_directory(&self, conversation_id: ConversationId) -> PathBuf {
        self.root_directory
            .join("conversations")
            .join(conversation_id.storage_key())
    }
}

fn create_private_directory(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
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

fn invalid_conversation_data(error: impl Error + Send + Sync + 'static) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

#[cfg(test)]
mod tests {
    use time::OffsetDateTime;

    use super::EventStore;
    use crate::conversation::{ConversationEventKind, ConversationId, UserContent};

    fn temporary_store() -> EventStore {
        let directory = std::env::temp_dir().join(format!("tog-test-{}", uuid::Uuid::now_v7()));
        EventStore::new(directory).expect("the event store should be created")
    }

    #[test]
    fn event_store_assigns_canonical_envelope_metadata() {
        let store = temporary_store();
        let conversation_id = ConversationId::new();

        let first_event = store
            .append_conversation_event(
                conversation_id,
                ConversationEventKind::User {
                    content: vec![UserContent::Text("first".to_owned())],
                },
            )
            .expect("the first event should be persisted");
        let second_event = store
            .append_conversation_event(
                conversation_id,
                ConversationEventKind::User {
                    content: vec![UserContent::Text("second".to_owned())],
                },
            )
            .expect("the second event should be persisted");

        assert_eq!(first_event.conversation_id, conversation_id);
        assert_eq!(first_event.position, 0);
        assert_eq!(first_event.schema_version, 9);
        assert_ne!(first_event.timestamp, OffsetDateTime::UNIX_EPOCH);
        assert_eq!(second_event.conversation_id, conversation_id);
        assert_eq!(second_event.position, 1);
        assert_eq!(second_event.schema_version, 9);
        assert_ne!(second_event.timestamp, OffsetDateTime::UNIX_EPOCH);
        assert_ne!(second_event.id, first_event.id);

        let conversation = store
            .load_conversation(conversation_id)
            .expect("the conversation should load");
        assert_eq!(conversation.events()[0].position, 0);
        assert_eq!(conversation.events()[1].position, 1);
        assert!(
            conversation
                .events()
                .iter()
                .all(|event| event.conversation_id == conversation_id)
        );
        assert!(
            !store
                .conversation_directory(conversation_id)
                .join("conversation.json")
                .exists()
        );
    }
}
