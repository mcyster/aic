# Conversation and ModelDriver Architecture

**Status:** Phase 1 design  
**Purpose:** Define a simple, durable conversation model and a narrow `ModelDriver` boundary that can be implemented against the OpenAI Responses API now and can support switching models/providers within a conversation.

This design is intentionally incomplete.

Phase 1 is not trying to build a perfect event-sourcing framework, a durable provider-protocol log, a distributed runtime, or a universal multi-provider SDK.

The priority is:

> Get the semantic conversation model and ModelDriver boundary right first.

The most important Phase 1 invariant is:

> Every ModelDriver must be able to continue a conversation using the Conversation Log alone, regardless of which ModelDriver produced the earlier events.

People are expected to change models during a conversation far more often than they are expected to replay historical provider protocol streams.

Where uncertain:

> Prefer a simple semantic contract now and preserve room for richer tracing and provider-native optimizations later.

---

## 1. Architectural overview

Phase 1 has one durable semantic event stream:

```text
Conversation Log
    semantic history
    durable replay source
    cross-model / cross-provider contract
```

A `ModelDriver` consumes an immutable view of that conversation and produces semantic conversation events:

```text
Conversation
    ↓
ModelDriver.invoke(...)
    ↓
ConversationEvents
    ↓
append to Conversation
```

Provider-specific details such as OpenAI Responses events, response IDs, token timing, reasoning protocol state, and HTTP diagnostics are **not part of the Phase 1 semantic replay contract**.

They may be captured through logging/tracing for observability.

Later, concrete benefits may justify making some of that provider-specific information durable, but semantic replay must not depend on it.

---

# Conversation model

## 2. Conversation

A conversation is a durable entity with an append-only stream of semantic `ConversationEvent`s.

Conceptually:

```rust
struct Conversation {
    id: ConversationId,
    created_at: DateTime<Utc>,
}
```

The event stream begins when something happens.

Phase 1 does not require conversation creation itself to be event zero.

The Conversation Log answers:

> What happened in the conversation?

It is the durable source for:

```text
ModelDriver input
CLI projection
model switching
semantic replay
automation
search/indexing
future UIs
```

---

## 3. ConversationEvent

Conversation events describe facts, not provider transport mechanics.

Conceptually:

```rust
enum ConversationEvent {
    User(...),
    ModelNote(...),
    ToolRequest(...),
    ToolResponse(...),
    AssistantResponse(...),
    ModelSpecific(...),
    Context(...),
    Automation(...),
    Data(...),
    Error(...),
}
```

The vocabulary should grow only when a concrete repeated semantic need justifies another event type.

OpenAI Responses events such as `response.created`, text deltas, and function argument deltas are not themselves conversation events.

---

## 4. Commands and events

Commands represent intent:

```text
CreateConversation
PostUserInput
InvokeModelDriver
ExecuteTool
MutateContext
SetData
PostAutomation
```

Conversation events represent facts:

```text
Command
    something should happen

ConversationEvent
    something happened
```

Commands are not part of canonical conversation history.

Do not force every command into:

```text
handle(command) -> Vec<Event>
```

Model invocation, tools, and external I/O naturally involve streaming, failures, and incremental output.

---

## 5. Strongly typed identifiers

Durable entities and cross-event references use strongly typed UUIDv7 identifiers.

Conceptually:

```rust
struct ConversationId(Uuid);
struct ConversationEventId(Uuid);
struct ToolCallId(Uuid);
struct ImageId(Uuid);
struct FileId(Uuid);
```

More typed IDs should be introduced when a concrete durable entity requires one.

The compiler should prevent accidental substitution of one identifier type for another.

Serialized forms should include a type prefix where practical:

```text
conversation_019...
conversation_event_019...
tool_call_019...
image_019...
file_019...
```

The verbosity is intentional. Explicit IDs are easier for humans and models to distinguish and reduce accidental or guessed references.

UUIDv7 ordering is useful for locality and diagnostics but is not authoritative replay ordering.

---

## 6. Event positions and replay order

Identity and ordering solve different problems.

Each stored conversation event has a monotonically increasing position:

```rust
struct StoredConversationEvent {
    position: u64,
    id: ConversationEventId,
    timestamp: DateTime<Utc>,
    schema_version: u32,
    event: ConversationEvent,
}
```

- `id` gives stable identity
- `position` gives authoritative replay order
- `timestamp` records observed wall-clock time
- `schema_version` permits persisted-format evolution

Phase 1 may assume a single writer and use a simple position allocator.

We do not need locks, distributed sequencing, compare-and-append, or a global event clock yet.

The invariant is simply:

> Replay the Conversation Log in stored position order.

Future persistence implementations may strengthen atomic allocation without changing the semantic model.

---

## 7. Semantic relationships are not ordering

Semantic relationships use typed IDs rather than stream positions.

For example:

```rust
struct ToolResponse {
    tool_call_id: ToolCallId,
    // ...
}
```

The `ToolCallId` identifies which request the response answers.

The event position identifies when the response entered the conversation.

Phase 1 does not require a generic causal graph or arbitrary predecessor relationships.

---

## 8. User content and external blobs

`User` records user-provided input.

A user event may contain multiple content parts:

```rust
enum UserContent {
    Text(String),
    Image(ImageId),
    File(FileId),
}

struct User {
    content: Vec<UserContent>,
}
```

Large or binary content should not be embedded directly in the Conversation Log.

Instead:

```text
store image/file/blob
    ↓
obtain strongly typed durable ID
    ↓
append User event containing the ID
```

Example:

```text
User
    Text("what is in this image?")
    Image(image_019...)
```

This keeps conversation events small and lets content storage, retention, permissions, deduplication, and provider transport evolve independently.

The ModelDriver resolves referenced content into whatever provider-specific representation is required.

Phase 1 only needs the concrete content types we actually use.

A failed model invocation never removes the already-durable `User` event.

---

## 9. ModelNote

`ModelNote` records model-produced information that is semantically useful to the conversation but is not the final assistant response.

It can support user-facing or model-relevant notes when useful.

It must not become a copy of every provider transport event.

Whether a note is displayed is a presentation decision.

---

## 10. AssistantResponse

`AssistantResponse` records the semantic assistant response produced by an invocation.

Provider transport may involve many low-level events:

```text
text.delta "Hel"
text.delta "lo"
output.done
```

but the semantic conversation records:

```text
AssistantResponse("Hello")
```

A ModelDriver may internally use streaming to construct or emit the response, but another ModelDriver does not need to understand that provider's streaming protocol.

---

## 11. ToolRequest

`ToolRequest` records that a model requested a tool invocation.

Each request has a stable `ToolCallId`:

```rust
struct ToolRequest {
    id: ToolCallId,
    // name
    // arguments
    // ...
}
```

A ToolRequest is a semantic fact.

It does not execute the tool itself.

The caller/runtime owns execution.

---

## 12. ToolResponse

`ToolResponse` records the result of one tool request and references exactly one `ToolCallId`:

```rust
struct ToolResponse {
    tool_call_id: ToolCallId,
    // result / error / metadata
}
```

A response is appended to the Conversation Log as soon as it arrives.

No batch abstraction is required.

---

## 13. Multiple tool requests

A single ModelDriver invocation may produce zero, one, or many `ToolRequest`s:

```text
ModelDriver invocation
    ↓
ToolRequest(A)
ToolRequest(B)
ToolRequest(C)
```

The caller may execute them sequentially or concurrently.

Responses are appended as they arrive:

```text
ToolResponse(B)
ToolResponse(A)
ToolResponse(C)
```

The core model does not prescribe:

```text
tool concurrency
response batching
when reinvocation occurs
whether all tool responses must arrive first
```

The caller owns that policy.

Phase 1 commits only to stable correlation through `ToolCallId`.

---

## 14. Context

`Context` records state that may affect later model invocation.

Examples:

```text
instructions
working directory
selected files
project
permissions
environment information intentionally exposed to the model
```

Context is distinct from user input.

---

## 15. Automation

`Automation` records information contributed by an external or asynchronous actor.

It is distinct from `ToolResponse`, which answers a model-requested tool invocation.

---

## 16. Data

`Data` records durable machine-readable metadata associated with the conversation.

Examples:

```text
external IDs
usage summaries
annotations
tags
diagnostics
UI metadata
```

Data is not model input by default.

---

## 17. ModelSpecific

`ModelSpecific` is a deliberate semantic escape hatch.

It represents model/provider-specific information that is useful enough to retain in the semantic conversation but does not yet justify a universal event type.

It should remain relatively rare.

This lets a ModelDriver preserve something genuinely useful without forcing the conversation model to predict every future provider capability.

Raw provider protocol events still belong in tracing/diagnostics rather than the semantic Conversation Log.

---

## 18. Error

`Error` records a failure that is semantically relevant to the conversation.

A failed invocation may therefore leave:

```text
User(...)
ModelNote(...)
ToolRequest(...)
ToolResponse(...)
Error(...)
```

depending on what happened before the failure.

Events that have already been appended are not rolled back.

The Error event should contain useful conversation-level information without exposing every provider/runtime diagnostic.

Detailed diagnostics belong in tracing/logging.

---

# ModelDriver

## 19. Why ModelDriver is an explicit abstraction

Phase 1 intentionally introduces a narrow `ModelDriver` abstraction even though only one provider is initially implemented.

This is deliberate.

The abstraction defines:

> What is the rest of the system allowed to know about model invocation?

Its purpose is to keep OpenAI Responses concepts out of the conversation, caller, CLI, and orchestration layers while we learn the new API.

This is an explicit exception to the normal preference against speculative abstraction.

The guardrail is:

> Keep ModelDriver very small and let concrete implementations pressure its shape.

Do not build:

```text
provider capability matrices
generic feature negotiation
large associated-type frameworks
universal provider event enums
provider inheritance hierarchies
```

A second driver should be allowed to reshape the abstraction.

---

## 20. Cross-driver semantic contract

Every ModelDriver must be able to invoke using only the semantic conversation state supplied in `ModelDriverInput`.

This is the central portability rule.

For example:

```text
User
OpenAI AssistantResponse
User
Anthropic AssistantResponse
ToolRequest
ToolResponse
User
Gemini ...
```

must be a valid conversation.

A driver must not require prior turns to have been produced by itself.

Provider-native continuation may later improve fidelity or performance, but it must remain optional.

Correctness and model switching are based on semantic ConversationEvents.

---

## 21. ModelDriverInput

`ModelDriverInput` is an immutable invocation snapshot.

Conceptually:

```rust
struct ModelDriverInput {
    conversation_id: ConversationId,
    conversation: ConversationSnapshot,
    model: ModelId,
    config: ModelConfig,
}
```

The precise fields should emerge from implementation.

Passing:

```rust
&ModelDriverInput
```

means the driver cannot mutate ordinary owned data through that reference.

For normal Rust-owned values such as structs, enums, `String`, and `Vec`, that provides the desired deep immutability through the borrowed input.

Interior-mutability types such as:

```text
Mutex
RwLock
RefCell
Atomic*
```

can still mutate behind `&T`, so input snapshots should avoid them unless there is a demonstrated need.

The intended contract is:

> A ModelDriver receives an immutable semantic snapshot and produces new facts rather than mutating historical conversation state.

A `conversation_id` is an identity reference, not mutable access to the stored conversation.

---

## 22. ModelDriver invocation

The exact `invoke` shape is intentionally still under iteration.

A useful starting point is:

```rust
trait ModelDriver {
    async fn invoke(
        &self,
        input: &ModelDriverInput,
        events: &mut ConversationEventSink,
    ) -> Result<ModelDriverResult, ModelDriverError>;
}
```

The exact event-emission mechanism may be a callback, sink, stream, iterator, channel, or another idiomatic Rust shape.

The important Phase 1 properties are:

- one call represents one model invocation
- input is immutable
- semantic conversation events may be emitted incrementally
- emitted events can be appended immediately
- the caller owns the outer model/tool loop
- success is strongly typed
- expected failures are strongly typed
- provider SDK types do not cross the boundary

We expect to iterate directly on this interface during implementation.

---

## 23. Immediate event persistence

Semantic events are appended as they happen.

A ModelDriver invocation is not a transaction whose partial output disappears if `invoke()` later returns `Err`.

Conceptually:

```text
User already durable
    ↓
invoke ModelDriver
    ↓
ModelNote appended
    ↓
ToolRequest appended
    ↓
later provider failure
    ↓
Error appended
    ↓
invoke returns Err
```

`Result` communicates the outcome of the operation.

It does not define a rollback boundary for conversation history.

This preserves the existing useful guarantee that a failed turn still participates in future reconstruction.

---

## 24. ModelDriverResult

Do not use:

```rust
Result<()>
```

as the public invocation contract.

The caller should receive a strongly typed result that tells it how the invocation ended at the control-flow level.

Conceptually:

```rust
struct ModelDriverResult {
    status: ModelDriverStatus,
}
```

with a small implementation-driven vocabulary, perhaps:

```rust
enum ModelDriverStatus {
    Completed,
    NeedsCallerAction,
}
```

The exact variants should be driven by actual OpenAI behavior.

The result should not duplicate detailed semantic facts already represented by ConversationEvents.

For example, tool requests remain events rather than being copied wholesale into `ModelDriverResult`.

The result exists so the caller knows how to react without understanding provider details.

---

## 25. ModelDriverError

Expected invocation failures are explicit in the function type:

```rust
Result<ModelDriverResult, ModelDriverError>
```

A small error model might begin with:

```rust
enum ModelDriverError {
    Authentication(...),
    RateLimited(...),
    Transport(...),
    InvalidRequest(...),
    Provider(...),
    Persistence(...),
}
```

The exact taxonomy should remain small and implementation-driven.

Rust does not use Java-style checked exceptions or `throws` declarations.

Expected operational failures are represented through `Result<T, E>`.

Unexpected programming failures may panic, but transport, provider, validation, persistence, and similar failures should normally be represented by `ModelDriverError`.

---

# Replay and model switching

## 26. Semantic replay is the Phase 1 correctness contract

Phase 1 reconstructs model input from the Conversation Log.

Conceptually:

```text
ConversationEvents
    ↓
ModelDriver-specific translation
    ↓
provider request
```

OpenAI translates semantic events to OpenAI Responses input.

Anthropic translates the same semantic events to Anthropic content/messages.

Gemini translates the same semantic events to its own representation.

No ModelDriver may require another provider's raw protocol history.

---

## 27. Model switching

Switching models/providers in the middle of a conversation is a first-class expected behavior, not an edge case.

For example:

```text
User
AssistantResponse produced by OpenAI
User
AssistantResponse produced by another ModelDriver
User
...
```

The new ModelDriver reads the semantic conversation and continues from it.

Some provider-specific fidelity may be lost when switching.

That is acceptable.

The semantic Conversation Log is the portability boundary.

---

## 28. Provider-native continuation

Provider-native continuation is explicitly **not required for Phase 1 correctness**.

For example, OpenAI may offer response IDs or reasoning-state mechanisms that improve same-provider continuation.

These may later be used as optimizations:

```text
lower token usage
higher reasoning fidelity
lower latency
better continuation
```

But the fallback must remain:

```text
semantic Conversation Log
    ↓
fresh provider request
```

A driver should never become unable to continue merely because provider-native history is unavailable.

---

# Caller/runtime

## 29. Outer orchestration loop

ModelDriver represents one model invocation.

It does not own the whole autonomous loop.

The caller owns orchestration:

```text
Conversation
    ↓
ModelDriver.invoke()
    ↓
0..N semantic events
    ↓
perhaps ToolRequests
    ↓
caller executes tools however it chooses
    ↓
ToolResponses appended as they arrive
    ↓
caller decides when to invoke ModelDriver again
```

This keeps concurrency, batching, scheduling, retry, and tool policy outside the ModelDriver abstraction.

---

## 30. Pending tool work

Runtime logic may derive pending tool work from semantic conversation state.

For example:

```text
ToolRequest(A)
ToolResponse(A) absent
    → A may require execution
```

while:

```text
ToolRequest(A)
ToolResponse(A) present
    → no pending response for A
```

Phase 1 needs only enough of this logic to support basic tool round-tripping.

It does not need a general workflow engine.

---

# CLI

## 31. CLI is a projection of the Conversation Log

The CLI consumes semantic conversation events.

It does not consume OpenAI transport events directly.

Conceptually:

```text
ModelDriver
    ↓
ConversationEvents
    ↓
Conversation Log
    ↓
CLI projection
```

This keeps the CLI independent of provider implementation.

A future interactive experience may also consume tracing/progress signals, but those do not redefine the semantic conversation contract.

---

# OpenAI Phase 1

## 32. OpenAI implementation strategy

Phase 1 implements `ModelDriver` against the OpenAI Responses API.

The goal is partly implementation and partly architectural discovery.

We want firsthand experience with the newer Responses model before deciding how much to rely on a provider-neutral Rust library.

OpenAI remains an implementation detail behind ModelDriver.

A later implementation may be:

```text
AnthropicModelDriver
GeminiModelDriver
GenAiModelDriver backed by rust-genai
another direct provider integration
```

and may cause the trait to evolve.

---

## 33. OpenAI ModelDriver responsibilities

The OpenAI implementation owns:

```text
Responses API request construction
semantic ConversationEvent → OpenAI input translation
OpenAI SDK / HTTP interaction
stream parsing
text aggregation
tool-call translation
OpenAI response IDs
reasoning/provider-specific protocol handling
provider errors
```

No OpenAI SDK/API type crosses the ModelDriver boundary.

The implementation emits semantic conversation events rather than exposing raw OpenAI protocol events to the caller.

---

## 34. Phase 1 OpenAI scope

Support enough to exercise the semantic architecture:

```text
basic text input/output
Responses API invocation
streaming response consumption
AssistantResponse
ModelNote where useful
function/tool requests
tool responses
multiple tool requests
typed success/error behavior
semantic reconstruction from Conversation Log
```

Out of scope unless nearly free:

```text
durable raw provider-event archival
provider-native replay as a correctness dependency
hosted web search
file search
computer use
image generation
background execution
```

Image/file input may be added when needed through typed content references.

---

# Observability and tracing

## 35. Phase 1 tracing

Provider-specific detail is useful even though it is not part of the semantic replay contract.

Phase 1 may trace/log:

```text
provider
model
request/response IDs
latency
time to first token/event
usage/token counts
raw or structured provider events
tool-call protocol activity
error diagnostics
HTTP/provider metadata
```

This information is primarily for:

```text
debugging
performance analysis
cost analysis
understanding provider behavior
development of the ModelDriver abstraction
```

It does not need to be represented as ConversationEvents.

It does not need to be replayable.

It does not need to be durable for correctness.

Existing tracing infrastructure should be preferred before introducing a separate durable event bus.

---

## 36. Raw provider events

Raw provider events may be extremely useful while learning the Responses API.

Capture them through tracing/logging when practical.

For example:

```text
response.created
output_item.added
reasoning events
output_text.delta
function_call_arguments.delta
response.completed
```

The important distinction is:

```text
ConversationEvent
    semantic product behavior

provider trace event
    implementation/diagnostic behavior
```

Phase 1 should not introduce semantic complexity merely to make raw provider protocol history replayable.

---

## 37. Tracing does not constrain ModelDriver implementations

A new ModelDriver should be straightforward to write.

At a high level, an implementation should need to:

```text
1. translate semantic conversation to provider input
2. call the provider
3. interpret provider output
4. emit semantic ConversationEvents
5. return a typed outcome/error
6. optionally emit useful traces
```

It should not need to implement:

```text
a timeless deterministic projection algebra
durable provider-log schema migration
cross-version re-projection
provider-log replay compatibility
global projection identities
```

Those requirements should only appear later if concrete value justifies them.

Ease of writing ModelDrivers is a first-class design constraint.

---

# Future direction

## 38. Durable ModelDriver run history

A future phase may introduce a durable ModelDriver run/event log if concrete needs justify it.

Potential motivations include:

```text
crash recovery inside partially completed model invocations
higher-fidelity same-provider replay
reasoning-state preservation
provider-native continuation
forensic debugging
long-running/background model work
distributed execution
```

A possible future shape is:

```text
ModelDriver invocation
    ↓
durable ModelDriverRun
    ↓
durable provider/run events
    ↓
observability / recovery / native replay
```

But that future log must remain supplemental to the semantic contract.

The invariant should remain:

> A ModelDriver can always continue from the Conversation Log alone.

---

## 39. Future event buses

Tracing and semantic emission may later become explicit buses/streams with multiple subscribers.

For example:

```text
ModelDriver
    ├── semantic ConversationEvent stream
    └── observability/provider event stream
```

Potential subscribers:

```text
conversation persistence
CLI/UI progress
metrics
tracing
debug logging
provider-native cache
future durable run history
```

Phase 1 does not need messaging infrastructure.

A local function call, sink, callback, or tracing span is sufficient.

The seam matters more than the machinery.

---

## 40. Future provider-native optimizations

When concrete evidence shows value, a ModelDriver may retain provider-native information to improve same-provider continuation.

Examples:

```text
OpenAI response IDs
reasoning state
provider cache references
uploaded file handles
provider-specific conversation IDs
```

These should be treated as caches/optimizations around the semantic conversation, not the only representation of history.

Switching providers must remain possible without them.

---

## 41. Future content storage

Conversation events should continue to reference large/binary content through durable typed IDs.

A future content store may add:

```text
content-addressed storage
deduplication
remote object storage
retention
access control
lazy materialization
provider upload caches
```

The stable contract remains:

```text
ConversationEvent
    references content ID

content store
    owns bytes/lifecycle

ModelDriver
    resolves content for provider invocation
```

---

## 42. Future replay and concurrency

The Phase 1 per-conversation position is sufficient for deterministic semantic replay.

Later requirements may motivate:

```text
global append positions
projection cursors
concurrent writers
optimistic append
transactional sequence allocation
explicit causal relationships
```

Those are persistence/runtime concerns.

They should not change the distinction between:

```text
typed identity
conversation replay order
semantic correlation
```

---

# Security

## 43. Phase 1 security baseline

Conversation context, tool output, content references, and provider traces may contain sensitive information.

Phase 1 does not need a complete redaction/retention system.

It should:

- avoid knowingly persisting obvious credentials
- avoid intentionally capturing environment secrets
- use private filesystem permissions for local durable conversation data
- be cautious when tracing raw provider payloads
- document that diagnostic logs may contain sensitive content

More complete security and retention policy belongs to a later phase.

---

# Phase 1 boundaries

## 44. What Phase 1 commits to

Phase 1 commits to:

```text
Conversation Log as the durable semantic truth

every ModelDriver can work from Conversation alone

model/provider switching within a conversation

a narrow explicit ModelDriver abstraction

immutable ModelDriverInput snapshots

strongly typed ModelDriverResult and ModelDriverError

strongly typed UUIDv7 identities

monotonic ConversationEvent positions

ToolCallId correlation

multiple tool requests without batching policy

typed external references for images/files/blobs

immediate semantic event append

caller-owned orchestration loop

CLI as a projection of Conversation
```

These are the seams we do not want to need to undo.

---

## 45. What Phase 1 intentionally does not solve

Phase 1 does not require:

```text
durable ModelDriver run logs
durable raw provider-event logs
projection identities between two durable logs
cross-version provider-log reprojection
provider-native replay for correctness
distributed event buses
locks/concurrent append coordination
global event clocks
generic causal DAGs
exactly-once tool side effects
general workflow orchestration
universal provider event taxonomy
production-grade tracing retention
complete OpenAI Responses coverage
```

These may become useful later, but they should not burden the first ModelDriver implementation.

---

## 46. First implementation milestone

The first coherent implementation should prove:

```text
CreateConversation

append User("hello")

construct immutable ModelDriverInput from Conversation

invoke OpenAiModelDriver

consume OpenAI Responses stream internally

emit AssistantResponse

append AssistantResponse immediately

return typed ModelDriverResult

reload Conversation

invoke OpenAiModelDriver again using only semantic Conversation history

switch to another ModelDriver later without requiring OpenAI run history
```

Then add:

```text
0..N ToolRequests
tool execution
ToolResponses appended as they arrive
caller-driven reinvocation
failure → Error
content references as concrete use cases require
useful provider tracing
```

The objective is to prove semantic portability and the ModelDriver boundary before adding provider-native sophistication.

---

# Documentation

## 47. Relationship to ADR 0001

The early `ModelDriver` abstraction is a deliberate exception to the normal rule against speculative provider abstractions.

Its purpose is not to predict a universal provider API.

Its purpose is to define and protect the model-invocation boundary while implementing the first provider.

Keep it small.

Let future implementations change it.

ADR 0001 or related architecture notes should explicitly record this rationale so the exception is deliberate rather than accidental.

---

## 48. Documentation status

This document is a **Phase 1 architectural direction**, not a finished permanent API.

Implementation experience should feed back into it.

If OpenAI Responses exposes assumptions that conflict with the design, document those pressures rather than hiding them behind increasingly elaborate abstractions.

The shorter `docs/conversation.md` should describe the stable semantic conversation model and should not imply that a durable ModelDriver run log is required for Phase 1.

Major future architectural changes should be captured in ADRs where useful.
