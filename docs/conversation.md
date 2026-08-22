# Conversation Model

The Conversation Log is `tog`'s durable semantic history. It records what happened in a conversation without exposing the provider protocol or the mechanics used to invoke a model.

The [Conversation and ModelDriver Architecture](conversation-design.md) is authoritative for model invocation, provider events, replay strategies, and Phase 1 implementation boundaries. This document summarizes the conversation concepts that should remain stable across those details.

## Conversation

A conversation begins with its first `ConversationEvent`. `Conversation` is an immutable in-memory projection reconstructed from the ordered events carrying one `ConversationId`:

```text
Conversation
    id
    events

Conversation Log
    position 0: ConversationEvent
    position 1: ConversationEvent
    ...
```

There is no independently persisted conversation record and no empty persisted conversation. Construction rejects an empty event sequence, mixed conversation IDs, and invalid event order. The projection exposes read-only access to its ID and events.

The Conversation Log answers:

> What happened in the conversation?

It is the stable source for semantic replay and consumers such as the CLI, automation, search, and future user interfaces.

## Completeness

The Conversation Log must be self-contained with respect to model-visible semantic state. Everything visible to a model, including content, instructions, context, model-visible tool descriptions and schemas, tool requests and responses, and referenced files or images, must be present in the log or immutably referenced by it.

Executable tool implementations, provider credentials, transport configuration, retry policy, and orchestration remain external. Their model-visible descriptions and schemas do not: those must be recorded or immutably referenced so a compatible `ModelDriver` can construct its request from the conversation alone.

## Event-Sourced State

The Conversation Log event-sources the semantic conversation domain. It is the authoritative source of semantic conversation state, and its events record durable semantic facts. The current state or view of a conversation is derived by replaying the log and resolving any immutable content it references; it must not depend on a separately mutable representation.

New semantic state is introduced by appending events, never by modifying earlier events. Derived projections, indexes, summaries, and provider requests may be rebuilt from the log and its immutable references.

This does not mean that all of `tog` is event-sourced. Commands, provider transport events, raw streaming deltas, retries, diagnostics, and execution mechanics are not automatically conversation events. Tracing or a separate operational log may capture those details without making them part of the canonical Conversation Log.

## Events Are Facts

Conversation events describe facts, not intent:

```text
Command
    something should happen

ConversationEvent
    something happened
```

Commands such as `PostUserInput`, `InvokeModelDriver`, and `ExecuteTool` are not part of canonical conversation history. Provider transport events are not conversation events either.

The intended semantic vocabulary is:

```text
User
Model
ToolRequest
ToolResponse
Context
Automation
Data
Error
```

The vocabulary should grow only when a repeated semantic need justifies another event type.

## Layered Semantic Representation

Conversation events define a universal semantic minimum and permit lossless enrichment beyond that minimum. Every compatible `ModelDriver` must understand the minimum, may interpret recognized enrichment for richer or more efficient continuation, and must safely ignore enrichment it does not understand. The conversation is therefore portable without being restricted to the lowest common denominator.

Driver-specific data enriches portable semantics; it must not replace them. The portable representation must contain enough information for another compatible driver to continue meaningfully, and semantically important structured concepts remain structured. For example, a tool request retains its portable call ID, tool name, and arguments even if it also contains a provider-specific call ID.

Driver-specific details remain immutable parts of the Conversation Log. Extensions should identify their owning driver or namespace and schema version when interpretation could otherwise be ambiguous. Raw provider transport events do not become semantic conversation events merely because they are provider-specific.

## Event Meanings

### User

`User` records user-provided input. One event may contain multiple content parts, such as text with an image or file.

Large or binary content belongs in a content store and is referenced by a strongly typed durable ID. Conversation events should not embed large payloads directly.

### Model

`Model` combines a typed model-produced `ModelEvent` with the `ModelSource` that produced it. `ModelSource` contains validated provider and model identities. Provenance belongs to the canonical `ConversationEvent`, not the `ModelEvent`, because the caller knows which driver it invoked and records that source on every yielded event. Full invocation configuration is not repeated on each event.

`AssistantResponse` is the model's actual response to the conversation. It participates in portable continuation and is always `Important`. `ModelCommunication` records auxiliary model-produced information such as detailed reasoning, reasoning summaries, status, or emerging concepts that do not yet justify another typed variant. Communications are persisted but are not automatically replayed as assistant responses.

Communication importance has three ordered levels: `Detailed`, `Interesting`, and `Important`. Consumers decide which messages to present, and the CLI maps low, medium, and high verbosity to progressively broader levels. Repeated cross-driver concepts may later be promoted from `ModelCommunication` into explicit `ModelEvent` variants.

Both typed variants retain meaningful portable messages and may carry extensions. Extensions provide driver-specific enrichment but do not replace the portable contract and may be ignored when not understood. Exposed reasoning is aggregated into coherent communications rather than persisting every transport delta.

For example, several provider events may project to one response:

```text
text.delta "Hel"
text.delta "lo"
output.done
    -> Model(source=..., event=AssistantResponse(message="Hello"))
```

### ToolRequest And ToolResponse

`ToolRequest` records that a model requested a tool invocation. Each request has a stable `ToolCallId`.

`ToolResponse` records the result of one request and references exactly one `ToolCallId`. A response is appended when it arrives, so response order does not need to match request order.

These events record semantic facts. They do not prescribe whether tools run sequentially or concurrently, or when the model is invoked again. The caller owns that orchestration policy.

### Context

`Context` records state that may affect later model invocation, such as instructions, working directory, selected files, project, or permissions. Context is distinct from user input.

### Automation

`Automation` records information contributed by an external or asynchronous actor. It is distinct from `ToolResponse`, which answers a model-requested tool invocation.

### Data

`Data` records durable machine-readable metadata such as external IDs, usage summaries, annotations, tags, or diagnostics. It is not model input by default.

### Error

`Error` records a failure that is semantically relevant to the conversation. It should contain useful conversation-level information without exposing all provider or runtime diagnostics.

## Identity, Order, And Relationships

Durable entities and references use strongly typed UUIDv7 identifiers. Distinct types prevent accidental substitution, for example:

```text
ConversationId
ConversationEventId
ToolCallId
ImageId
FileId
```

Each conversation event also has a monotonically increasing stream position. Identity, order, and semantic relationships serve different purposes:

- the conversation ID identifies the conversation to which the event belongs
- the event ID provides stable identity
- the position provides authoritative replay order within the conversation
- the timestamp records observed wall-clock time but does not determine order
- typed references such as `ToolCallId` express semantic relationships

Event positions must not be used as semantic identifiers.

## Durability And Projection

User input is appended before model invocation. The asynchronous invocation establishes one provider/model request and returns a stream of completed semantic `ModelEvent`s. The consumer controls demand by polling that stream for its next event; receiving several events from it does not represent several model requests.

The caller may combine each yielded event with the invoked driver's `ModelSource`, assign persistence metadata, display it, and append the resulting `ConversationEvent` while the invocation remains active. If the stream later fails, completed semantic events already yielded remain valid conversation facts and appended events are not rolled back. Provider deltas that did not form a completed `ModelEvent` are discarded. The caller receives the stream error and decides whether to append a semantic conversation-level `Error` event.

This supersedes the earlier batch contract in which all model events were returned only after the complete invocation succeeded and all model output was discarded on a late provider failure. A caller that needs batch behavior can collect the stream; no separate batch interface is required.

Provider-native state may later improve same-provider continuation, but the Conversation Log remains the durable representation used for local reconstruction and cross-provider replay.

## System Boundary

The conversation model deliberately excludes:

- provider request construction and transport
- raw provider events
- model invocation lifecycle and diagnostics
- tool execution policy
- retry and scheduling policy
- CLI rendering and interactive progress

Those concerns consume, produce, or project conversation events without becoming part of the semantic model. Their detailed boundaries are defined in [Conversation and ModelDriver Architecture](conversation-design.md).
