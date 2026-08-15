# Conversation and ModelDriver Architecture

**Status:** Phase 1 design  
**Purpose:** Define a strong event-oriented architecture for `tog` that can be implemented against the OpenAI Responses API now, while preserving enough information and clean enough boundaries to evolve without a restart later.

This design is intentionally incomplete.

Phase 1 is not trying to build a perfect event-sourcing framework, a distributed runtime, or a universal multi-provider SDK. It is trying to establish the seams we are least willing to undo later.

The guiding principle is:

> Strong direction, simple implementation, preserved information, easy evolution.

Where uncertain:

> Preserve information and keep the boundary open.

---

## 1. Architectural overview

There are two durable event streams with different purposes:

```text
Conversation Log
    semantic history

ModelDriver Run Log
    operational / provider history
```

The distinction is fundamental.

### Conversation Log

Answers:

> What happened in the conversation?

Examples include:

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

This is the stable semantic surface used by consumers such as the CLI, semantic replay, automation, search, and future UIs.

### ModelDriver Run Log

Answers:

> What happened while a model invocation was being performed?

Examples include:

```text
RunStarted
ProviderEvent
RunCompleted
RunFailed
```

This is where provider-specific activity belongs, including raw OpenAI Responses events.

The broad flow is:

```text
Command
   ↓
ModelDriver Run
   ↓
durable ModelDriverRunEvents
   ↓
projection
   ↓
durable ConversationEvents
   ↓
CLI / replay / automation / other projections
```

Phase 1 may implement these boundaries with direct function calls and local sinks. No actual messaging infrastructure is required.

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

Phase 1 does not require creation itself to be represented as event zero.

The event stream begins when something happens in the conversation.

This keeps the model simpler without preventing a future `ConversationCreated` event if creation later becomes semantically useful.

---

## 3. ConversationEvent

Conversation events are facts, not commands.

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

The exact Rust payload types should emerge from implementation, but these semantic categories are the intended Phase 1 vocabulary.

A `ConversationEvent` is deliberately distinct from a provider event. OpenAI-specific Responses events do not belong directly in this enum.

---

## 4. Commands

Commands represent intent.

Examples:

```text
CreateConversation
PostUserInput
InvokeModelDriver
ExecuteTool
MutateContext
SetData
PostAutomation
```

Commands are not part of the canonical conversation history.

The conceptual distinction is:

```text
Command
    something should happen

ModelDriverRun
    a durable attempt to invoke a model

ConversationEvent
    something happened in the conversation
```

Do not force all command handling into:

```text
handle(command) -> Vec<Event>
```

That shape does not naturally model streaming, failures, incremental persistence, external I/O, or concurrency.

For Phase 1, simple commands may be handled synchronously while model invocation is explicitly streaming/effectful.

---

## 5. Strongly typed identifiers

All durable entities and cross-event references should use strongly typed identifiers.

Use UUIDv7 internally, wrapped in Rust newtypes.

Conceptually:

```rust
struct ConversationId(Uuid);
struct ConversationEventId(Uuid);

struct ModelDriverRunId(Uuid);
struct ModelDriverRunEventId(Uuid);

struct ToolCallId(Uuid);
```

The compiler should prevent accidental substitution of one identifier type for another.

Serialized forms should include a type prefix where practical:

```text
conversation_019...
conversation_event_019...
model_driver_run_019...
model_driver_run_event_019...
tool_call_019...
```

The verbosity is intentional. Explicitly typed IDs are easier for humans and models to distinguish and reduce accidental or guessed cross-references.

UUIDv7 provides useful approximate temporal locality, but UUID ordering is not authoritative replay ordering.

---

## 6. Event positions and replay order

Identity and ordering solve different problems.

Every durable event stream should assign a monotonically increasing position.

Conceptually:

```rust
struct StoredEvent<T, Id> {
    position: u64,
    id: Id,
    timestamp: DateTime<Utc>,
    schema_version: u32,
    event: T,
}
```

Properties:

- `id`: stable identity and cross-reference
- `position`: authoritative deterministic replay order within the stream
- `timestamp`: observed wall-clock time, not ordering authority
- `schema_version`: persisted schema evolution

Phase 1 may assume a single writer and use a trivial position allocator.

We do not need locks, distributed sequencing, optimistic append, or a global clock yet.

The invariant we do want now is:

> Persistence assigns a monotonic position within each durable stream and replay occurs in position order.

Future implementations may strengthen how that position is allocated atomically without changing the domain model.

---

## 7. Semantic relationships are not ordering

Domain relationships should use typed IDs, not stream positions.

For example:

```rust
struct ToolResponse {
    tool_call_id: ToolCallId,
    // ...
}
```

The `ToolCallId` tells us which request the response belongs to.

The event position tells us when it was appended.

Similarly, projected conversation events use ModelDriver run/event IDs for provenance.

Phase 1 does not require a generic causal DAG or arbitrary `previous_event_id` relationships.

Those can be introduced later if branching or explicit causality becomes important.

---

## 8. User

`User` represents user-provided input.

The conversation log should reference external content rather than embed large binary payloads directly.

For example:

```rust
struct ImageId(Uuid);
struct FileId(Uuid);

enum UserContent {
    Text(String),
    Image(ImageId),
    File(FileId),
}

struct User {
    content: Vec<UserContent>,
}
```

A single `User` event may carry multiple content parts, for example text alongside an image. This is why `content` is a `Vec<UserContent>` rather than a single `UserContent`.

The conceptual flow is:

```text
store image/file/blob
    ↓
obtain strongly typed durable ID
    ↓
append User event containing that ID
```

For example:

```text
User
    Text("what is in this image?")
    Image(image_019...)
```

This keeps conversation events small, avoids duplicating binary content during replay, and lets the content store, retention, deduplication, permissions, and transport evolve independently from the conversation model.

The `ModelDriver` is responsible for resolving referenced content into the provider-specific representation needed for invocation.

Phase 1 only needs text and whatever minimal image/file referencing is required by concrete use cases. The architectural rule is that large binary content lives outside the conversation event log and is referenced by typed ID.

A failed model invocation does not remove the already-durable `User` event. The user turn remains part of future reconstruction.

---

## 9. ModelNote

`ModelNote` represents model-produced information that is semantically useful to the conversation but is not the final assistant response.

This can be used for model-visible/user-visible notes when the provider exposes information we choose to preserve semantically.

It should not become a dumping ground for every raw provider event. Raw provider details remain in the ModelDriver Run Log.

Whether a note is shown to a user is a projection/UI decision.

---

## 10. AssistantResponse

`AssistantResponse` represents semantic assistant output added to the conversation.

It is not equivalent to every piece of output emitted by a model invocation.

For example:

```text
ModelDriverRunEvent
    response.created

ModelDriverRunEvent
    text.delta "Hel"

ModelDriverRunEvent
    text.delta "lo"

ModelDriverRunEvent
    output.done

        ↓ semantic projection

ConversationEvent
    AssistantResponse("Hello")
```

The run log preserves the detailed provider history.

The conversation log preserves the semantic result.

---

## 11. ToolRequest

`ToolRequest` records that the model requested a tool invocation.

Each request has a stable `ToolCallId`:

```rust
struct ToolRequest {
    id: ToolCallId,
    // name
    // arguments
    // ...
}
```

A `ToolRequest` is a conversation fact.

It is not itself the imperative operation that executes the tool.

The caller/runtime may derive:

```text
ExecuteTool(tool_call_id)
```

from conversation state.

The naming distinction is intentional:

```text
ToolRequest
    semantic conversation event

ToolCallId
    stable correlation identity
```

---

## 12. ToolResponse

`ToolResponse` records the result of executing one tool request.

It references exactly one `ToolCallId`:

```rust
struct ToolResponse {
    tool_call_id: ToolCallId,
    // result / error / metadata
}
```

The relationship is explicit and durable.

A tool response is appended to the conversation as soon as it arrives.

No batch object is required.

---

## 13. Multiple tool requests

A single ModelDriver invocation may produce zero, one, or many `ToolRequest`s.

For example:

```text
ModelDriver invocation
    ↓
ToolRequest(A)
ToolRequest(B)
ToolRequest(C)
```

The caller/runtime may execute them sequentially or concurrently.

Responses are appended as they arrive:

```text
ToolResponse(B)
ToolResponse(A)
ToolResponse(C)
```

The core model does not impose a concurrency or reinvocation policy.

Phase 1 commits only to:

- 0..N tool requests may come from one invocation
- every request has its own `ToolCallId`
- every response references exactly one `ToolCallId`
- responses may be appended immediately in completion order
- the caller/runtime decides when to invoke the `ModelDriver` again

This keeps the conversation model simple and keeps orchestration policy outside it.

---

## 14. Context

`Context` represents state that may affect subsequent model invocation.

Examples:

```text
instructions
working directory
selected files
environment
project
permissions
```

Context is distinct from user input.

The important distinction is:

```text
Context
    may contribute to future model input

Data
    metadata about the conversation
    not model input by default
```

---

## 15. Automation

`Automation` represents an external or asynchronous actor contributing something to the conversation.

This is distinct from a tool response.

For example:

```text
ToolResponse
    response to a model-requested tool

Automation
    independent process contributes new information
```

This distinction was useful in the prior conversation model and remains useful here.

---

## 16. Data

`Data` represents durable machine-readable metadata associated with the conversation.

Examples:

```text
external IDs
cost
usage summaries
annotations
UI metadata
tags
diagnostics
```

`Data` is not included in model context by default.

---

## 17. ModelSpecific

`ModelSpecific` is an intentional semantic escape hatch.

It represents model/provider-specific information that is useful enough to retain at the conversation level but does not yet justify a universal semantic event type.

It should remain relatively rare.

Raw provider protocol events still belong in the ModelDriver Run Log.

`ModelSpecific` exists so the semantic model can evolve without either losing useful information or prematurely inventing generic abstractions.

---

## 18. Error

A model invocation failure can be semantically relevant to the conversation and should therefore be representable as:

```text
ConversationEvent::Error(...)
```

The operational failure remains in the ModelDriver Run Log as `RunFailed`.

The two representations have different purposes:

```text
ModelDriverRunEvent::RunFailed
    operational/provider diagnostics

ConversationEvent::Error
    semantic failure for conversation projections
```

The semantic error should contain useful conversation-level information without requiring all provider/runtime diagnostics to leak into the conversation.

---

# ModelDriver

## 19. Why ModelDriver is an explicit abstraction

Phase 1 intentionally introduces a narrow `ModelDriver` abstraction even though only one concrete provider is initially implemented.

This is deliberate.

The abstraction is not primarily justified by “we might support Anthropic later.”

It is justified by the need to define:

> What is the rest of the system allowed to know about model invocation?

Without this boundary, OpenAI Responses concepts can easily leak into conversation, runtime, CLI, and projection code while we are still learning the new API.

This is a deliberate exception to the usual “avoid speculative abstraction” rule.

The guardrail is:

> Keep `ModelDriver` extremely small and let the concrete OpenAI implementation pressure its shape.

Do not build:

```text
provider capability matrices
generic feature negotiation
provider inheritance hierarchies
large associated-type frameworks
universal normalized event enums
```

A later second implementation should be allowed to reshape the abstraction rather than merely conform to it.

This decision should be documented as an explicit architectural exception to ADR 0001 rather than silently violating it.

---

## 20. ModelDriver interface

The `ModelDriver` invocation boundary should make input immutability and caller-visible outcomes explicit.

Conceptually:

```rust
trait ModelDriver {
    async fn invoke(
        &self,
        input: &ModelDriverInput,
        events: &mut ModelDriverRunEventSink,
    ) -> Result<ModelDriverResult, ModelDriverError>;
}
```

The exact Rust signature is not prescribed, but this is the intended shape.

### Immutable invocation input

`ModelDriverInput` represents an immutable invocation snapshot.

It should contain stable identities and the information required for this invocation, for example:

```rust
struct ModelDriverInput {
    conversation_id: ConversationId,
    run_id: ModelDriverRunId,

    model: ModelId,
    conversation: ConversationSnapshot,
    config: ModelConfig,
}
```

The precise fields will emerge during implementation.

Passing:

```rust
&ModelDriverInput
```

means the driver cannot mutate the input through that reference.

For ordinary owned Rust fields such as structs, enums, `String`, and `Vec`, this gives the desired deep immutability through the borrowed value.

Interior-mutability types such as:

```text
Mutex
RwLock
RefCell
Atomic*
```

can still mutate state behind an immutable reference, so `ModelDriverInput` should avoid them unless there is a specific demonstrated reason.

The intended contract is:

> A ModelDriver invocation receives a stable immutable snapshot. It emits new facts through the run-event sink rather than mutating its input.

`conversation_id` and `run_id` are references to durable identities. They do not imply mutable access to the stored Conversation or ModelDriverRun.

### Strongly typed invocation result

Do not use:

```rust
Result<()>
```

as the public invocation contract.

The caller should receive a strongly typed result that tells it how the invocation ended at the API/control-flow level.

Conceptually:

```rust
struct ModelDriverResult {
    run_id: ModelDriverRunId,
    status: ModelDriverStatus,
}
```

with a small status vocabulary such as:

```rust
enum ModelDriverStatus {
    Completed,
    NeedsCallerAction,
}
```

The exact variants should be driven by real OpenAI implementation needs.

The result should not duplicate detailed semantic facts already persisted in the event logs. For example, tool requests and assistant responses remain durable events rather than being copied wholesale into `ModelDriverResult`.

The result exists so the caller can react without inspecting provider-specific details.

### Strongly typed errors

Expected invocation failures are represented explicitly:

```rust
Result<ModelDriverResult, ModelDriverError>
```

with a typed error model approximately like:

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

Recoverable/expected failures are part of the function type through `Result<T, E>`.

Unexpected programmer failures may panic, but provider, transport, validation, persistence, and similar operational failures should normally be represented by `ModelDriverError`.

This makes the error contract explicit in the type system.

### Invocation responsibilities

The important properties remain:

- invocation is one model-driver call, not the entire autonomous tool loop
- input is an immutable snapshot
- run events are emitted incrementally
- the caller owns the outer loop
- the caller receives a typed success/outcome value
- expected failures are typed
- OpenAI SDK types do not cross the boundary
- provider-specific replay logic belongs behind the boundary

The precise `invoke` contract is expected to receive further iteration during Phase 1.

---

# ModelDriver Runs

## 21. ModelDriverRun

A `ModelDriverRun` represents a durable attempt to invoke a model driver.

Conceptually:

```rust
struct ModelDriverRun {
    id: ModelDriverRunId,
    conversation_id: ConversationId,
    created_at: DateTime<Utc>,
    // invocation/request snapshot
}
```

The exact persisted representation may evolve.

The distinction is:

```text
InvokeModelDriver
    transient intent

ModelDriverRun
    durable attempt

ModelDriverRunEvents
    durable operational/provider history

ConversationEvents
    semantic consequences
```

Phase 1 should create a durable `ModelDriverRun` before external model I/O begins.

Phase 1 may initially use one `ModelDriverRun` per ModelDriver invocation.

Do not generalize it into a whole agent/workflow run yet.

---

## 22. ModelDriverRunEvent

A run has its own durable append-only event stream.

Phase 1 needs only a small vocabulary:

```rust
enum ModelDriverRunEvent {
    RunStarted(...),
    ProviderEvent(...),
    RunCompleted(...),
    RunFailed(...),
}
```

Future versions may add:

```text
RetryStarted
RetryCompleted
Checkpoint
ToolStarted
ToolCompleted
```

if real requirements make those useful.

---

## 23. Run event boundary

The architecture should behave as though `ModelDriverRunEvent`s are emitted onto an event stream or bus.

Phase 1 does not require messaging infrastructure.

A local synchronous/async sink is sufficient.

Conceptually:

```rust
trait ModelDriverRunEventSink {
    fn emit(&mut self, event: ModelDriverRunEvent) -> Result<(), ModelDriverError>;
}
```

`emit` reuses the same `ModelDriverError` taxonomy introduced in §20 (for example `Persistence`) so the run-event sink carries the same typed-error discipline as `invoke`.

The seam is important now.

The infrastructure behind it can remain trivial.

A likely Phase 1 flow is:

```text
create ModelDriverRun
    ↓
append RunStarted
    ↓
call OpenAI
    ↓
receive provider event
    ↓
append ProviderEvent
    ↓
project semantic conversation output
    ↓
receive next provider event
```

---

## 24. Durable run lifecycle

A durable `ModelDriverRun` must exist before the external provider call begins.

This closes the crash window where a request is sent but no provider event has yet been received.

Conceptually:

```text
InvokeModelDriver
    ↓
create ModelDriverRun
    ↓
append RunStarted
    ↓
external OpenAI call
```

Phase 1 does not need sophisticated transaction semantics between creation and `RunStarted`.

The important guarantee is that local invocation identity and request information exist before external I/O.

The run should retain enough invocation configuration to understand or reconstruct what was attempted.

At minimum:

```text
provider
model
instructions/configuration
tool definitions or references
input/replay strategy
```

The precise request snapshot format may evolve.

---

## 25. Raw provider events

Raw provider events belong in the ModelDriver Run Log.

For OpenAI Responses, these may include events such as:

```text
response.created
output_item.added
reasoning-related events
output_text.delta
function_call_arguments.delta
response.completed
...
```

Do not immediately collapse them into a universal model event enum.

Use an open representation conceptually like:

```rust
struct ProviderEvent {
    provider: ProviderId,
    model: String,
    event_type: String,
    payload: Value,
}
```

wrapped by:

```text
ModelDriverRunEvent::ProviderEvent(...)
```

No OpenAI SDK-specific type should cross the `ModelDriver` boundary.

---

## 26. Preserve provider payloads

Preserve provider payloads as faithfully as practical.

Where feasible, persistence should happen from the original provider/SSE payload before an SDK representation has discarded unknown information.

SDKs may:

```text
drop unknown fields
normalize structures
rename values
lag newly introduced provider fields
```

“Raw” does not require byte-for-byte archival in Phase 1.

It means:

> Avoid unnecessary information loss.

---

## 27. Provider response identity

Provider-side IDs should be captured when available.

For OpenAI, this includes response IDs.

Local `ModelDriverRunId` remains authoritative inside `tog`.

Provider IDs are external references useful for continuation, diagnostics, and correlation.

---

# Projection and provenance

## 28. Projection philosophy

The core abstraction over provider events is not a universal event taxonomy.

Instead, stored history supports several projections:

```text
ModelDriver native replay
Conversation semantic projection
runtime/control projection
CLI/UI projection
```

Projection may transform, coalesce, ignore, or interpret events.

It is not simply a set of booleans such as:

```text
send_to_model = true
show_to_user = true
```

---

## 29. Semantic projection

A durable `ModelDriverRunEvent` may derive zero or more semantic `ConversationEvent`s.

Examples:

```text
OpenAI response.created
    → nothing semantic

reasoning summary completed
    → perhaps ModelNote

completed function call
    → ToolRequest

completed assistant output
    → AssistantResponse

provider-specific useful semantic output
    → ModelSpecific

run failure
    → Error
```

Projection operates from durable stored run events, not untracked transient provider events.

---

## 30. ProjectionIdentity

Every conversation event derived from ModelDriver run history must retain stable provenance.

Use a projection identity conceptually like:

```rust
struct ProjectionIdentity {
    source_run_id: ModelDriverRunId,
    source_run_event_id: ModelDriverRunEventId,
    output_index: u32,
}
```

`output_index` allows one source run event to produce more than one semantic event.

For example:

```text
ModelDriverRunEvent X
    ↓
ModelNote           output_index = 0
ToolRequest         output_index = 1
AssistantResponse   output_index = 2
```

Projection provenance is cross-cutting storage metadata and should live in the stored conversation-event envelope rather than being repeated in every event payload.

Conceptually:

```rust
struct StoredConversationEvent {
    position: u64,
    id: ConversationEventId,
    timestamp: DateTime<Utc>,
    schema_version: u32,
    projection: Option<ProjectionIdentity>,
    event: ConversationEvent,
}
```

Direct user/automation events normally have:

```text
projection = None
```

Events derived from run history have:

```text
projection = Some(...)
```

---

## 31. Projection API must preserve provenance

Avoid an API like:

```rust
emit(ConversationEvent)
```

for semantic output derived from a run event.

A naked event loses the source identity needed for idempotent crash recovery.

For Phase 1, a deliberately simple shape is preferable:

```rust
struct ProjectedConversationEvent {
    output_index: u32,
    event: ConversationEvent,
}

fn project(
    source: &StoredModelDriverRunEvent,
) -> Vec<ProjectedConversationEvent>;
```

The commit layer combines:

```text
source ModelDriverRunId
source ModelDriverRunEventId
output_index
```

into `ProjectionIdentity`.

The exact implementation can vary, but provenance should be structurally difficult to forget.

---

## 32. Cross-log consistency

The ModelDriver Run Log and Conversation Log are persisted independently.

A process may crash after a source run event is durable but before the derived semantic event is written.

Correctness should not require a transaction spanning both logs.

Instead:

> Semantic projection must be reproducible and idempotent.

Persistence treats `ProjectionIdentity` as unique.

Conceptually:

```text
UNIQUE (
    source_run_id,
    source_run_event_id,
    output_index
)
```

or an equivalent invariant.

Example:

```text
ModelDriverRunEvent X: output.done
    persisted

CRASH

AssistantResponse not persisted
```

Recovery:

```text
reload run events
    ↓
re-run projector
    ↓
derive AssistantResponse(output_index=0)
    ↓
ProjectionIdentity(X, 0) absent
    ↓
append
```

If it was already committed:

```text
re-run projector
    ↓
derive same ProjectionIdentity
    ↓
already exists
    ↓
skip
```

On restart, incomplete or relevant runs may therefore be reprojected safely.

This preserves the crash-recovery behavior that already exists in the previous system.

---

## 33. Immediate persistence

Events are committed as they happen.

A ModelDriver invocation is not a transaction whose partial history disappears when `invoke()` returns `Err`.

The invariant is:

> Once an event has happened and been durably appended, a later failure does not roll it back.

Conceptually:

```text
append User
    ↓
invoke ModelDriver
    ↓
append ModelDriverRunEvents incrementally
    ↓
project + append semantic ConversationEvents incrementally
    ↓
failure occurs
    ↓
append RunFailed
    ↓
project + append Error
    ↓
return Err
```

`Result` communicates the invocation outcome.

It does not define a persistence transaction boundary.

Avoid:

```text
invoke
    ↓
collect staged events
    ↓
success → commit
error   → discard
```

because that loses already-produced history and weakens crash recovery.

---

# Replay

## 34. Replay has two forms

Provider continuation and local reconstruction are distinct mechanisms.

### Provider-side continuation

For example, OpenAI may support:

```text
previous_response_id
```

This relies on provider-retained state and provider-specific semantics.

### Local reconstruction

The application constructs a new invocation from durable local history:

```text
ConversationEvents
ModelDriverRunEvents
stored request/configuration
tool history
```

Provider continuation is useful, but local durability must not assume provider state is permanent or portable.

---

## 35. Same-provider replay

When continuing with the same provider and sufficient native history is available:

```text
Conversation Log
+
ModelDriver Run history
    ↓
provider-native projection
    ↓
new provider request
```

The OpenAI ModelDriver owns the rules for reconstructing or continuing OpenAI state.

A critical invariant is:

> Do not replay both provider-native output and the semantic event derived from that same output.

For example:

```text
raw OpenAI output items
+
AssistantResponse derived from those items
```

must not both become duplicate model input.

The ModelDriver chooses one authoritative representation.

---

## 36. Cross-provider replay

A different provider cannot necessarily consume OpenAI-native events or reasoning state.

Cross-provider replay therefore uses the semantic Conversation Log.

Conceptually:

```text
User
ModelNote
ToolRequest
ToolResponse
AssistantResponse
Context
ModelSpecific where portable/meaningful
Error where appropriate
```

becomes input translated by the new ModelDriver.

Thus:

```text
same provider
    native/high-fidelity replay where useful

different provider
    semantic conversation replay
```

Phase 1 only implements OpenAI, but the separation is intentional.

---

# Runtime/control

## 37. Runtime projection

Runtime logic may derive pending work from durable conversation state.

For example:

```text
ToolRequest(tool_call_A)
ToolResponse(tool_call_A) absent
    ↓
tool A may require execution
```

and:

```text
ToolRequest(tool_call_A)
ToolResponse(tool_call_A) present
    ↓
no pending response for A
```

This operates over conversation history rather than an isolated event.

Phase 1 only needs enough runtime logic for tool round-tripping.

It does not need a generalized scheduler or workflow engine.

---

## 38. Outer orchestration loop

`ModelDriver` represents one model invocation.

It does not own the entire autonomous loop.

The caller/runtime owns orchestration:

```text
Conversation
    ↓
ModelDriver.invoke()
    ↓
0..N semantic outputs, perhaps ToolRequests
    ↓
caller executes tools however it chooses
    ↓
ToolResponses appended as they arrive
    ↓
caller decides when to invoke ModelDriver again
```

This deliberately avoids embedding concurrency or batching policy into the driver or conversation model.

---

# CLI

## 39. CLI is a projection of the Conversation Log

The normal CLI surface is derived from semantic `ConversationEvent`s.

It is not driven directly by raw provider events.

Conceptually:

```text
ModelDriverRunEvents
    ↓
semantic projection
    ↓
ConversationEvents
    ↓
CLI projection
```

For example:

```text
text.delta "Hel"
text.delta "lo"
output.done

    ↓

AssistantResponse("Hello")

    ↓

CLI output
```

The CLI decides how semantic events map to stdout/stderr and interactive presentation.

The important architectural point is:

> The CLI consumes the conversation model, not the OpenAI transport stream.

---

## 40. Future interactive presentation

A richer interactive renderer may eventually combine:

```text
ConversationEvents
+
selected ModelDriverRunEvents
```

to show progress such as:

```text
reasoning summaries
model activity
tool progress
streaming output
status
```

That does not change the Conversation Log or make raw provider events semantic conversation events.

This can evolve independently as another projection.

---

# OpenAI Phase 1

## 41. OpenAI implementation strategy

Phase 1 implements `ModelDriver` directly against the OpenAI Responses API.

The purpose is partly architectural discovery.

We want firsthand experience with the newer event-oriented API before deciding how much to rely on a multi-provider Rust library.

This does not make OpenAI the domain model.

OpenAI remains an implementation behind `ModelDriver`.

A later implementation may be:

```text
AnthropicModelDriver
GeminiModelDriver
GenAiModelDriver backed by rust-genai
another direct provider integration
```

and may force useful changes to the abstraction.

---

## 42. OpenAI ownership

The OpenAI ModelDriver owns:

```text
Responses API request construction
OpenAI SDK / HTTP interaction
SSE event handling
raw provider event capture
OpenAI response IDs
reasoning/state handling
native replay
provider continuation
tool-call protocol translation
```

No OpenAI SDK/API type crosses the `ModelDriver` boundary.

---

## 43. Phase 1 OpenAI scope

Support enough to exercise the architecture:

```text
basic text input
Responses API invocation
streaming response events
raw provider-event persistence
response IDs
usage
reasoning-related state where available
function/tool requests
tool responses
continuation/reinvocation
```

Do not attempt full Responses API coverage.

Out of scope unless nearly free:

```text
hosted web search
file search
computer use
image generation
background execution
```

---

## 44. Incremental provider persistence

Provider events should be persisted incrementally as they arrive.

Conceptually:

```text
receive provider event
    ↓
assign ModelDriverRunEventId
assign run position
    ↓
persist
    ↓
project semantic output if now complete
    ↓
continue stream
```

Do not wait until the complete model response before recording operational history.

This gives:

```text
crash visibility
debuggability
deterministic replay
provider archaeology
future projection flexibility
```

---

# Security

## 45. Phase 1 security baseline

Raw provider events, context, and tool output may contain sensitive information.

Examples:

```text
credentials
tokens
environment variables
private file contents
provider metadata
tool output
user data
```

A complete redaction, retention, and permission system is outside Phase 1.

However, Phase 1 should:

- avoid knowingly persisting obvious credentials
- avoid intentionally capturing environment secrets
- persist local conversation/run data with private filesystem permissions
- document that raw event persistence may contain sensitive data

Security hardening remains a future requirement, not a reason to block the initial architecture.

---

# Phase 1 boundaries

## 46. What Phase 1 commits to

Phase 1 commits to these architectural seams:

```text
ConversationEvents as semantic history

ModelDriverRunEvents as operational/provider history

a narrow explicit ModelDriver abstraction

immutable ModelDriverInput snapshots

strongly typed ModelDriverResult and ModelDriverError

strongly typed UUIDv7 IDs

monotonic per-stream replay positions

raw/open provider payload preservation

immediate durable append

ProjectionIdentity on derived semantic events

idempotent cross-log projection and recovery

ToolCallId correlation for multiple tool requests

typed external references for images/files/blobs

provider-native versus semantic replay

caller-owned orchestration loop

CLI as a projection of the Conversation Log
```

These are the decisions we want to avoid needing to undo.

---

## 47. What Phase 1 intentionally does not solve

Phase 1 does not require:

```text
distributed event buses
locks or concurrent append coordination
global event clocks
generic causal DAGs
cross-log distributed transactions
exactly-once tool side effects
tool execution attempt journals
universal normalized provider event taxonomy
perfect cross-provider replay
general workflow orchestration
complete failure taxonomy
production-grade redaction/retention
complete OpenAI Responses coverage
```

These are future extensions, not missing prerequisites.

---

## 48. First implementation milestone

The first coherent implementation should prove:

```text
CreateConversation

append User("hello")

InvokeModelDriver

create durable ModelDriverRun

append RunStarted

call OpenAI Responses through OpenAiModelDriver

receive provider events

persist each ModelDriverRunEvent immediately
with typed IDs and monotonic positions

project complete semantic outputs
with ProjectionIdentity

append AssistantResponse

append RunCompleted

reload persisted state

re-run semantic projection safely
without duplicate ConversationEvents

project Conversation to CLI

reinvoke OpenAI using the selected replay strategy
```

Then add:

```text
0..N ToolRequests
tool execution
ToolResponses appended as they arrive
caller-driven reinvocation
failure → RunFailed → Error
```

The objective is to validate the boundaries, not to build runtime sophistication.

---

# Future direction

## 49. Event-stream evolution

Phase 1 uses local calls/sinks while preserving event-stream boundaries.

The likely future shape is:

```text
Commands
    ↓
ModelDriver Run stream
    ↓
durable ModelDriverRunEvents
    ↓
multiple projections
    ├── Conversation Log
    ├── runtime/control
    ├── progress / interactive UI
    ├── observability
    └── provider-native replay
```

And:

```text
durable ConversationEvents
    ↓
multiple projections
    ├── CLI
    ├── interactive UI
    ├── semantic model replay
    ├── search/indexing
    └── automation
```

A synchronous call is sufficient today.

Future implementations may introduce:

```text
async event buses
multiple subscribers/projectors
projection cursors
background recovery
checkpoints
global stream positions
concurrent writers
optimistic append
richer ModelDriverRun lifecycles
cross-process orchestration
tool execution attempt journals
```

The current design should leave room for these without implementing them prematurely.

---

## 50. Projection future direction

The projection model is intentionally more important than a normalized provider event taxonomy.

As new providers and capabilities arrive, we should first ask:

```text
What should be returned to the model?
What should become semantic conversation state?
What should be shown to the user?
What should cause runtime action?
```

Only introduce new universal event types when repeated concrete implementations demonstrate that they are truly common.

`ModelSpecific` and raw `ProviderEvent` payloads provide escape hatches while the architecture learns.

---

## 51. ModelDriver future direction

The current `ModelDriver` trait is intentionally narrow.

Future providers may reveal that the initial shape is wrong.

That is acceptable.

The abstraction exists to protect the boundary and help us reason about the system, not to freeze a speculative universal provider API.

When a second implementation arrives:

1. compare its concrete needs with OpenAI
2. retain what is actually common
3. change the trait where necessary
4. avoid preserving an early shape merely for compatibility

The goal is a useful abstraction, not an untouchable one.

---

## 52. Content storage future direction

Conversation events should continue to reference large or binary content by strongly typed durable IDs rather than embedding it.

Future content storage may add:

```text
deduplication
content-addressed storage
remote object storage
retention policy
access control
lazy materialization
provider-specific upload caches
```

Those concerns should remain outside the semantic Conversation Log.

The stable contract is:

```text
ConversationEvent
    references content ID

content store
    owns bytes and content lifecycle

ModelDriver
    resolves referenced content for provider invocation
```

---

## 53. Replay and consistency future direction

The current per-stream monotonic position is sufficient for deterministic local replay.

Later, system-wide projections may motivate a global append position or cursor.

Concurrent writers may motivate compare-and-append or transactional sequence allocation.

Tool side-effect recovery may motivate a separate durable tool-attempt log with idempotency keys.

None of these require changing the fundamental distinction between:

```text
typed identity
stream replay order
semantic correlation
projection provenance
```

That distinction should remain stable.

---

# Documentation

## 54. Documentation status

This document is a **Phase 1 architectural direction**, not a permanent finished API.

Implementation experience should feed back into it.

If OpenAI Responses exposes assumptions that conflict with this model, document those pressures rather than hiding them behind increasingly elaborate abstractions.

Related architecture documents should be updated so they do not continue to describe these decisions as unresolved.

The explicit early `ModelDriver` abstraction should be reconciled with ADR 0001 as a deliberate exception: the abstraction is being used to define and protect a design boundary, not because a second provider already exists.

Add this document to the documentation index.

Major future architectural changes should be captured in ADRs where useful.
