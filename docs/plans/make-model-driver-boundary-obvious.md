# Make the Conversation and Model Boundary Obvious

## Goal

A `ConversationEvent` is a durable, portable fact. Every event may also carry
optional model-specific data without requiring that model's driver to understand
the conversation later.

The intended shape is approximately:

```rust
struct ConversationEvent {
    // identity, position, timestamp, and schema
    turn_id: ConversationTurnId,
    turn_boundary: TurnBoundary,
    kind: ConversationEventKind,
    model: Option<ModelData>,
}

enum TurnBoundary {
    Start,
    Continue,
    End,
}

enum ConversationEventKind {
    User { /* portable content */ },
    Model { source: ModelSource, event: ModelEvent },
    Problem { problem: ConversationProblem },
}
```

`Problem` remains a top-level conversation event. It is not model output merely
because it concerns a model invocation.

## Turns

A conversation turn starts with a user prompt and ends with either an assistant
response or a terminal problem. The boundary is recorded on that same
`ConversationEvent`:

- a user prompt starts the turn;
- intermediate model activity and recoverable problems continue it;
- an assistant response or terminal problem ends it.

The ending event's kind determines whether the turn responded or failed. An
assistant response creates one conversation event, not a response event followed
by a separate turn-completion event.

A `ConversationTurn` may be derived from the events sharing a turn identity. It
is not another persisted conversation message.

## Model data

`ModelData` is opaque to the Conversation and supports JSON serialization. The
driver that creates it defines and interprets its contents. Another driver may
ignore it safely.

The portable event kind must contain the complete meaning of the event.
`ModelData` may preserve native fidelity or improve continuation, but it must
not be required to understand the conversation. It must retain enough identity
for a driver to decide whether it knows how to interpret the JSON.

Model data is recorded when the event is created. Later drivers do not mutate
old events to attach their own representations.

There is no `Other` problem kind. A new understood semantic problem receives a
specific shared kind; unusable provider output and unclassified invocation
failure retain their distinct existing meanings.

## Driver boundary

A `ModelDriver` receives the portable Conversation and produces one stream of
semantic events. A model-reported problem is an event on that stream, not a
parallel output channel.

The driver translates provider-native activity into portable semantics and may
supply `ModelData`. The caller creates the durable envelope, adds provenance
and turn information, and persists it. `ModelDriverError` remains operational
control flow; the caller may also record an appropriate sanitized `Problem`.

Do not make a provider-native event the only durable record. Reconstructing a
conversation must never require the original driver or its historical version.

## Open details

The exact driver event type and how it supplies a portable event plus optional
`ModelData` remain to be designed. The shared `ConversationProblem` vocabulary
should be refined from concrete cases without adding speculative hierarchy.

## Complete when

The types visibly separate portable Conversation meaning, optional model data,
and turn boundaries. Problems remain top-level events, one event can end a turn,
and switching drivers never requires the previous driver to read the
conversation.
