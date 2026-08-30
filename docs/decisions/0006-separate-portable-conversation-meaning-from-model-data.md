# 0006: Separate Portable Conversation Meaning from Model Data

## Status

Accepted

## Context

Model-specific data lived inside the portable event vocabulary: `AssistantResponse` and `ModelCommunication` carried `extensions`, and `ModelIssue::Other` accepted arbitrary extension objects. The `Problem` kind also repeated the invoked `ModelSource`, which made problems look like model output merely because they concerned a model invocation. At the driver boundary, `ModelDriverOutput::Issue` ran as a second channel beside `ModelDriverOutput::Event`, so a model-reported problem was not visibly an event on the driver's stream.

These choices obscured the portability contract: another driver could not tell which parts of a durable event it must understand to continue the conversation.

## Decision

Every `ConversationEvent` carries a flattened portable `ConversationEventKind` plus an optional envelope-level `ModelData`:

```rust
struct ConversationEvent {
    // identity, position, timestamp, and schema
    kind: ConversationEventKind,
    model: Option<ModelData>,
}
```

`ModelData` is opaque to the conversation and serializes as JSON. The driver that creates it defines and interprets its contents; any other driver may ignore it safely. It retains its owning `ProviderId` so a driver can decide whether it knows how to interpret the content. Model data is recorded when the event is created; later drivers never mutate old events to attach their own representations.

The portable kind contains the complete meaning of the event. `AssistantResponse` and `ModelCommunication` no longer carry extensions. `Problem { problem: ConversationProblem }` remains a top-level conversation event without a `ModelSource`; `ModelProblem` is renamed `ConversationProblem`.

There is no `Other` problem kind. A newly understood semantic problem receives a specific shared `ModelIssue` kind, while unusable provider output (`InvocationError::InvalidProviderResponse`) and unclassified invocation failure (`InvocationError::ProviderFailure`) retain their distinct existing meanings.

A `ModelDriver` receives the portable `Conversation` and produces one stream of `ModelDriverEvent` values: a portable `ModelEvent` or a model-reported problem (`ModelIssue`), each optionally accompanied by `ModelData`. `ModelDriverError` remains operational control flow; the caller may also record an appropriate sanitized `Problem`. The caller creates the durable envelope, adds provenance, and persists it.

## Consequences

Portable conversation meaning and optional model data are visibly separate, and switching drivers never requires the previous driver to read the conversation. Reconstructing a conversation never requires the original driver or its historical version.

Events persist at schema version 10. Earlier problem events load without their source, and earlier events load without their extensions, because the conversation no longer models those fields. Events persisted with `ModelIssue::Other` no longer deserialize; no production driver ever emitted them.

This decision supersedes the extension mechanism described in ADR 0005 and restates its problem surface without `ModelSource`, and it renames the stream payload that ADR 0004 called `ModelDriverOutput`.
