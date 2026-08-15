# Architecture

`tog` is a single Cargo package containing one executable.

The command-line module translates process arguments into typed turn requests. The turn service coordinates semantic conversation events, durable agent-run events, and the OpenAI Responses provider. The executable writes semantic assistant output to standard output and conversation metadata to standard error.

Conversation and agent-run streams use separate local append-only logs. Each event is stored as an atomically renamed JSON file with a typed UUIDv7 identifier, monotonic stream position, timestamp, and schema version. Phase 1 assumes one writer per stream.

The first provider uses OpenAI's Responses API through a narrow HTTP/SSE adapter. Raw provider events are persisted before semantic projection. Completed model output becomes an idempotent conversation event referencing its source agent-run event. Subsequent turns use the prior OpenAI response ID when available and retain semantic history for local reconstruction.

The detailed design and implementation boundaries are in [Conversation Architecture](conversation.md).
