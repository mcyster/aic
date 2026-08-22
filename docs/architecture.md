# Architecture

`tog` is a single Cargo package containing one executable.

## The Conversation Is the Portable Immutable Record

A conversation is an immutable, append-only semantic representation of the conversation. Everything made visible to a model must be recorded in the conversation or immutably referenced by it. This includes user and model content, instructions, context, model-visible tool definitions, tool requests and responses, and referenced files or images.

`tog` is conversation-centered, and its semantic conversation domain is event-sourced. The append-only Conversation Log is the authoritative source of semantic conversation state. Current state and views are derived by replaying its durable semantic facts and resolving any immutable content they reference; they must not depend on a separately mutable representation. New semantic state is introduced by appending events, never by modifying earlier events. Projections, indexes, summaries, and provider requests may therefore be rebuilt from the log and its immutable references.

This event-sourcing boundary does not extend automatically to every operational detail in `tog`. Commands, provider transport events, raw streaming deltas, retries, diagnostics, and execution mechanics are not conversation events merely because they occur while processing a conversation. They may be captured through tracing or a separate operational log without becoming part of the canonical Conversation Log.

This completeness defines the `ModelDriver` boundary. A driver must be able to construct its provider request from the conversation without additional unrecorded model-visible input. Provider transport configuration, credentials, executable tool implementations, retry policy, and orchestration mechanics remain outside the conversation because they control invocation without adding semantic state visible to the model.

Another compatible `ModelDriver` must be able to continue the conversation without provider-native history or mutable state owned by the original driver. This supports semantic continuation and replay across providers; it does not promise deterministic reproduction of model output. Exact reconstruction of a historical provider request may additionally require durable invocation metadata such as the provider, model, parameters, and projection version.

Conversation events define a universal semantic minimum and permit lossless enrichment beyond that minimum. Every compatible driver must understand the minimum, may interpret recognized enrichment for richer or more efficient continuation, and must safely ignore enrichment it does not understand. Driver-specific data enriches portable semantics and never replaces them, so portability does not restrict events to the lowest common denominator.

Semantically important structured concepts remain structured in the portable representation. For example, a tool request retains its portable call ID, tool name, and arguments even when it also carries a provider-specific call ID. Extensions should identify their owning driver or namespace and schema version when interpretation could otherwise be ambiguous. Raw provider transport events do not become semantic conversation events merely because they are provider-specific.

The command-line module translates process arguments into typed turn requests. The turn service appends user input, passes an immutable view of semantic history to a configured `ModelDriver`, and appends the `ConversationEvent`s returned by a successful invocation. The executable filters model-event messages by CLI verbosity and writes them to standard output. Conversation metadata and operational failures use standard error.

The Conversation Log is the only durable event stream required for Phase 1 correctness. Each event is stored as an atomically renamed JSON file with a typed UUIDv7 identifier, monotonic conversation position, timestamp, and schema version. Phase 1 assumes one writer per conversation.

The first `ModelDriver` uses OpenAI's Responses API through a narrow HTTP/SSE adapter. It owns its configured model, translates the complete semantic conversation into provider input, consumes streaming protocol events internally, and returns semantic model events. Exposed reasoning is aggregated into detailed or interesting events; final responses are important events. Raw provider events and OpenAI response IDs are not persisted. Every invocation reconstructs provider input from the Conversation Log, so continuation does not depend on OpenAI history and another driver can continue the conversation.

The stable semantic concepts are summarized in the [Conversation Model](conversation.md). The detailed design and implementation boundaries are in [Conversation and ModelDriver Architecture](conversation-design.md).
