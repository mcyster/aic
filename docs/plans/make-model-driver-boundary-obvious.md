# Make the ModelDriver Boundary Obvious

The path from a provider invocation to durable conversation facts should be
understandable from the types without first reading the implementation or the
architecture documents.

The current behavior is coherent, but the public shape within the crate is not
yet self-explanatory:

```rust
enum ModelDriverOutput {
    Event(ModelEvent),
    Issue(ModelIssue),
}
```

`Event` hides the important distinction between an `AssistantResponse` and a
`ModelCommunication`. `Issue` appears to be something other than an event even
though the caller persists it as a top-level `ConversationEventKind::Problem`.
This is what prompted the question: where is the assistant response in the
`ModelDriver` output?

## Intended outcome

A third-party Rust developer evaluating `tog` should be able to see the
following boundary directly in its types and names:

- `Conversation` is portable, durable semantic history.
- One `ModelDriver::invoke` represents one provider/model invocation and yields
  completed semantic outputs incrementally.
- An assistant response is the model's user-facing successful result.
- Model communication is auxiliary successful output, not an assistant response.
- A model issue is an understood unsuccessful outcome.
- A `ModelDriverError` is detailed control flow; the turn layer records a
  sanitized invocation problem before returning it.
- The caller adds `ModelSource`, creates canonical event envelopes, persists
  them, and owns orchestration.

The output hierarchy should express those distinctions without duplicating
concepts between the driver and conversation layers.

## Settled boundaries

The review should not reopen these decisions without new evidence:

- `ConversationEventKind::Model` contains successful `ModelEvent` output.
- `ModelEvent` distinguishes `AssistantResponse` from
  `ModelCommunication`.
- `ConversationEventKind::Problem` is a top-level durable fact associated with
  the selected `ModelSource`.
- `ModelProblem` distinguishes a semantic `ModelIssue` from a sanitized
  invocation failure.
- Raw provider events, partial deltas, response identifiers, credentials, and
  detailed transport failures do not enter the Conversation Log.
- The outer turn service owns persistence, retry policy, cancellation, tools,
  and reinvocation.
- A higher-level `Agent` remains a future orchestration concept rather than the
  provider integration boundary.

These boundaries are described in
[Conversation and ModelDriver Architecture](../conversation-design.md) and the
reasoning is summarized in the
[conversation and ModelDriver note](../notes/2026-08-29-conversation-and-model-driver.md).

## Critical question

Decide whether the current `ModelDriverOutput` hierarchy merely needs clearer
names or whether it introduces an unnecessary layer.

In particular, compare:

- retaining the `ModelEvent` and `ModelIssue` split with an output envelope
  whose variants name those concepts directly;
- exposing a different completed-output vocabulary that makes assistant,
  communication, and issue immediately visible;
- removing the output envelope if the stream and `Result` can express the
  boundary more directly without collapsing semantic issues into operational
  errors.

The choice should preserve one obvious mapping from each driver output to one
canonical conversation fact.

## Work

Review the boundary as an external Rust library evaluator would, including
comparison with a small set of established Rust model or agent libraries.
Concentrate on ownership, asynchronous streaming, successful-output vocabulary,
and error semantics rather than feature counts.

Use that review to choose the smallest output model that makes the domain clear.
Then align the Rust types, the turn-service mapping, focused tests, and the
authoritative architecture documents. Remove superseded terminology rather than
documenting multiple equivalent paths.

Do not expand this work into tool execution, a durable provider run log,
provider-native continuation, or a general `Agent` abstraction.

## Complete when

The work is complete when a reader can answer these questions from the relevant
type definitions alone:

1. Where is the assistant's response?
2. Which outputs are successful conversation contributions?
3. Which unsuccessful outcomes become durable problems?
4. Which failures remain Rust control flow?
5. Who supplies provenance and persists the resulting conversation event?

The implementation, tests, and concise architecture documentation must give the
same answers.
