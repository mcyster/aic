# Conversation Model

The conversation is `tog`'s durable semantic history. It records what happened in a conversation without exposing the provider protocol or the mechanics used to invoke a model.

The [Conversation and ModelDriver Architecture](conversation-design.md) supersedes the earlier detailed version of this document. It is authoritative for ModelDriver runs, provider events, projection mechanics, replay strategies, and Phase 1 implementation boundaries. This document summarizes the conversation concepts that should remain stable across those details.

## Conversation

A conversation is a durable entity with an append-only stream of `ConversationEvent`s:

```text
Conversation
    id
    created_at

Conversation Log
    position 0: ConversationEvent
    position 1: ConversationEvent
    ...
```

Conversation creation does not need to be an event unless creation itself becomes semantically meaningful. The event stream begins when something happens.

The Conversation Log answers:

> What happened in the conversation?

It is the stable source for semantic replay and consumers such as the CLI, automation, search, and future user interfaces.

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
ModelNote
ToolRequest
ToolResponse
AssistantResponse
ModelSpecific
Context
Automation
Data
Error
```

The vocabulary should grow only when a repeated semantic need justifies another event type.

## Event Meanings

### User

`User` records user-provided input. One event may contain multiple content parts, such as text with an image or file.

Large or binary content belongs in a content store and is referenced by a strongly typed durable ID. Conversation events should not embed large payloads directly.

### ModelNote

`ModelNote` records model-produced information that is semantically useful but is not the final response. It must not become a copy of every provider event. Whether a note is visible to the user is a presentation decision.

### AssistantResponse

`AssistantResponse` records the semantic response produced by a model invocation. It is distinct from lower-level output deltas, reasoning events, usage, and lifecycle events.

For example, several provider events may project to one response:

```text
text.delta "Hel"
text.delta "lo"
output.done
    -> AssistantResponse("Hello")
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

### ModelSpecific

`ModelSpecific` is a limited escape hatch for provider-specific information that is semantically useful at the conversation level but does not justify a universal event type. Raw provider protocol events still belong outside the Conversation Log.

### Error

`Error` records a failure that is semantically relevant to the conversation. It should contain useful conversation-level information without exposing all provider or runtime diagnostics.

## Identity, Order, And Relationships

Durable entities and references use strongly typed UUIDv7 identifiers. Distinct types prevent accidental substitution, for example:

```text
ConversationId
ConversationEventId
ModelDriverRunId
ModelDriverRunEventId
ToolCallId
ImageId
FileId
```

Each stored event also has a monotonically increasing stream position. Identity, order, and semantic relationships serve different purposes:

- the event ID provides stable identity
- the position provides authoritative replay order within the conversation
- the timestamp records observed wall-clock time but does not determine order
- typed references such as `ToolCallId` express semantic relationships

Event positions must not be used as semantic identifiers.

## Durability And Projection

Events are appended as they happen. A later model or tool failure does not roll back facts that are already durable. For example, a `User` event remains in the conversation if the following model invocation fails.

Conversation events derived from ModelDriver run history retain stable projection provenance. This makes projection reproducible and idempotent: recovery can derive a missing semantic event again without duplicating one that was already stored.

Provider continuation and semantic replay are separate concerns. Provider-native state may improve same-provider continuation, but the Conversation Log remains the durable semantic representation used for local reconstruction and cross-provider replay.

## System Boundary

The conversation model deliberately excludes:

- provider request construction and transport
- raw provider events
- model invocation lifecycle and diagnostics
- tool execution policy
- retry and scheduling policy
- CLI rendering and interactive progress

Those concerns consume, produce, or project conversation events without becoming part of the semantic model. Their detailed boundaries are defined in [Conversation and ModelDriver Architecture](conversation-design.md).
