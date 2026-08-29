# Make the ModelDriver Boundary Obvious

## Goal

ModelDriver should produce one stream of semantic events. An issue is an event
on that stream, not a parallel kind of output.

Issues are first a Conversation concept:

```text
ConversationEvent
    → ConversationIssue
        → model-specific issue
```

The model may define several concrete issue kinds, such as refusal, context
exhaustion, or invocation failure.

## Current mismatch

`ModelDriverOutput::{Event, Issue}` says that an issue is not an event.
`ConversationEventKind::Problem` is also tied directly to `ModelSource` and
`ModelProblem`, so the supposedly general conversation concept is already
model-specific.

## Direction

Introduce a clear conversation-level issue concept and let model issues
specialize it. Keep one ModelDriver event stream, while retaining
`ModelDriverError` for operational Rust control flow.

The remaining design question is whether `ConversationIssue` should specialize
directly into `ModelIssue`, or whether a useful conversation-wide
classification belongs between them. Do not add generic levels such as warning,
failure, or limitation unless they enforce a concrete distinction.

Source, message, severity, and retryability should be modeled at the level where
their meaning and invariants belong. A future non-model issue must not be forced
to carry a `ModelSource`.

## Complete when

The Conversation issue hierarchy is evident from the types, model issues travel
on the single event stream, and the implementation, tests, and architecture
documents describe the same boundary.
