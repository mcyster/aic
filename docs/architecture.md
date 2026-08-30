# Architecture

`tog` is a single Cargo package containing one executable.

## The Conversation Is the Portable Immutable Record

The ordered events in the Conversation Log form an immutable, append-only semantic representation of a conversation. Everything made visible to a model must be recorded in those events or immutably referenced by them. This includes user and model content, instructions, context, model-visible tool definitions, tool requests and responses, and referenced files or images.

`tog` is conversation-centered, and its semantic conversation domain is event-sourced. The append-only Conversation Log is the authoritative source of semantic conversation state. Current state and views are derived by replaying its durable semantic facts and resolving any immutable content they reference; they must not depend on a separately mutable representation. New semantic state is introduced by appending events, never by modifying earlier events. Projections, indexes, summaries, and provider requests may therefore be rebuilt from the log and its immutable references.

### ConversationEvent And Conversation

`ConversationEvent` is the canonical durable semantic fact and the unit persisted, published, and carried on an event bus. Its `ConversationEventKind` defines which semantic fact occurred, while the event carries canonical identity, conversation membership, ordering, timestamp, and schema metadata, plus optional opaque `ModelData` supplied by the driver that created it. A conversation begins with its first event; there is no independently persisted mutable conversation record. Reconstructing the ordered events sharing a `ConversationId` reconstructs the conversation.

`Conversation` is an immutable in-memory projection of those ordered events, not a separate aggregate or source of truth. It may expose its ID and a vector or read-only sequence of events for convenient consumption, but all of its state is derived from the events. Construction validates that every event has the same `ConversationId` and that the sequence is in a valid order.

Individual `ConversationEvent`s are published to an event bus; `Conversation` itself is not. The primary model invocation flow is:

```text
immutable Conversation
    → asynchronous ModelDriver invocation
    → stream of completed ConversationEvents
    → caller persists and presents each returned event
    → ConversationEvents appended incrementally
```

One `ModelDriver::invoke` call represents one provider/model invocation. Its outer future establishes the invocation and returns a stream; the stream then yields a complete `ConversationEvent` or a typed error. The driver translates provider-native activity into the portable event kind, provenance, optional `ModelData`, and canonical envelope metadata. Provider-specific intermediate events remain private to the concrete driver. The caller persists or publishes each returned event. New conversation state exists only after those events are appended.

`ConversationEventKind::Problem` contains one authoritative `ConversationProblem`. `ConversationProblem::Issue` records a meaningful limitation or unsuccessful outcome understood by the driver, such as a refusal or exceeded context limit. `ConversationProblem::Invocation` records a sanitized operational failure. `ModelDriverError` remains the detailed Rust control-flow error; the turn service converts it into a durable invocation problem before returning it. The problem itself owns its message and retryability semantics; the conversation event does not duplicate message or severity fields.

This event-sourcing boundary does not extend automatically to every operational detail in `tog`. Commands, provider transport events, raw streaming deltas, retries, diagnostics, and execution mechanics are not conversation events merely because they occur while processing a conversation. They may be captured through tracing or a separate operational log without becoming part of the canonical Conversation Log.

This completeness defines the `ModelDriver` boundary. A driver must be able to construct its provider request from the reconstructed conversation without additional unrecorded model-visible input. Provider transport configuration, credentials, executable tool implementations, retry policy, and orchestration mechanics remain outside the conversation because they control invocation without adding semantic state visible to the model.

Another compatible `ModelDriver` must be able to continue the conversation without provider-native history or mutable state owned by the original driver. This supports semantic continuation and replay across providers; it does not promise deterministic reproduction of model output. Exact reconstruction of a historical provider request may additionally require durable invocation metadata such as the provider, model, parameters, and projection version.

Conversation events define a universal semantic minimum and permit lossless enrichment beyond that minimum. Every compatible driver must understand the minimum, may interpret recognized enrichment for richer or more efficient continuation, and must safely ignore enrichment it does not understand. Driver-specific data enriches portable semantics and never replaces them, so portability does not restrict events to the lowest common denominator.

Semantically important structured concepts remain structured in the portable representation. For example, a tool request retains its portable call ID, tool name, and arguments even when it also carries a provider-specific call ID. Driver-specific enrichment lives in the event envelope's optional `ModelData`, which retains its owning `ProviderId` so a driver can decide whether it can interpret the content. Raw provider transport events do not become semantic conversation events merely because they are provider-specific.

The command-line module translates process arguments into typed turn requests. The turn service appends user input, reconstructs and validates the `Conversation`, passes an immutable reference to a configured `ModelDriver`, and persists each returned conversation event as it is yielded. The executable filters model-event messages by CLI verbosity and writes them to standard output. Conversation identifiers and operational failures use standard error.

The Conversation Log is the only durable event stream required for Phase 1 correctness. Each event is stored as an atomically renamed JSON file carrying its `ConversationId`, typed UUIDv7 event identifier, monotonic conversation position, timestamp, and schema version. There is no separate conversation metadata file. Phase 1 assumes one writer per conversation.

The first `ModelDriver` uses OpenAI's Responses API through a narrow asynchronous HTTP/SSE adapter. One invocation uses one REST request and one SSE response stream. The driver owns a stable source containing provider `openai` and its configured model, translates the complete semantic conversation into provider input, keeps protocol events, raw text deltas, and intermediate `ModelDriverEvent`s private, aggregates provider deltas, and returns completed `ConversationEvent`s. Consuming several events does not make several model requests. Final output becomes an `AssistantResponse`; exposed reasoning and reasoning summaries become `ModelCommunication`s with their declared importance; refusals and recognized context-limit responses become top-level problem events. Known model-associated concepts have explicit contracts; the driver currently supplies no `ModelData`, and another driver's model data is ignored during replay. Raw provider events, raw provider error bodies, and OpenAI response IDs are not persisted as semantic facts. Every invocation reconstructs provider input from the Conversation Log, so continuation does not depend on OpenAI history and another driver can continue the conversation.

The stable semantic concepts are summarized in the [Conversation Model](conversation.md). The detailed design and implementation boundaries are in [Conversation and ModelDriver Architecture](conversation-design.md).
