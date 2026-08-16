# Architecture

`tog` is a single Cargo package containing one executable.

The command-line module translates process arguments into typed turn requests. The turn service appends user input, passes an immutable view of semantic history to a configured `ModelDriver`, and appends the `ConversationEvent`s returned by a successful invocation. The executable filters model-event messages by CLI verbosity and writes them to standard output. Conversation metadata and operational failures use standard error.

The Conversation Log is the only durable event stream required for Phase 1 correctness. Each event is stored as an atomically renamed JSON file with a typed UUIDv7 identifier, monotonic conversation position, timestamp, and schema version. Phase 1 assumes one writer per conversation.

The first `ModelDriver` uses OpenAI's Responses API through a narrow HTTP/SSE adapter. It owns its configured model, translates the complete semantic conversation into provider input, consumes streaming protocol events internally, and returns semantic model events. Exposed reasoning is aggregated into detailed or interesting events; final responses are important events. Raw provider events and OpenAI response IDs are not persisted. Every invocation reconstructs provider input from the Conversation Log, so continuation does not depend on OpenAI history and another driver can continue the conversation.

The stable semantic concepts are summarized in the [Conversation Model](conversation.md). The detailed design and implementation boundaries are in [Conversation and ModelDriver Architecture](conversation-design.md).
