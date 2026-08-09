# Conversation Architecture

**Status:** Phase 1 design
**Goal:** Establish a strong event-oriented architecture that can evolve without requiring a restart as model APIs, providers, and runtime behavior change.

This design is intentionally incomplete.

Phase 1 should prove the architecture against the OpenAI Responses API, preserve enough information to learn from newer event-oriented model APIs, and avoid premature provider-neutral generalization.

The priority is:

> Strong direction, simple implementation, preserved information, easy evolution.

We explicitly do **not** need a perfect event-sourcing framework, agent runtime, or provider abstraction in Phase 1.

---

## 1. Architectural overview

There are two durable event streams with different purposes.

```text
Conversation Log
    semantic history

Agent Run Log
    operational/model-runtime history
```

The distinction is fundamental.

### Conversation Log

Answers:

> What happened in the conversation?

Examples:

```text
User
Assistant
ToolCall
ToolResponse
Context
Automation
Data
```

This is the stable semantic surface used by consumers such as the CLI.

### Agent Run Log

Answers:

> What happened while the agent/runtime was producing conversation events?

Examples:

```text
RunStarted
ModelProviderEvent
RunCompleted
RunFailed
```

This is where raw provider activity belongs.

The runtime bridges the two:

```text
Command
   ↓
Agent Run
   ↓
AgentRunEvents
   ↓
semantic projection
   ↓
ConversationEvents
```

---

## 2. Conversation events

A conversation is an append-only stream of semantic `ConversationEvent`s.

Events describe things that happened.

They are facts, not commands.

Conceptually:

```text
ConversationEvent
    User
    Assistant
    ToolCall
    ToolResponse
    Context
    Automation
    Data
```

Phase 1 does not require conversation creation itself to be represented as event zero.

A conversation may simply exist as a durable entity:

```text
Conversation
    id
    created_at
```

with its event stream beginning when something happens.

This avoids forcing `ConversationCreated` into the event model merely for event-sourcing purity.

We may revisit this later if creation itself becomes semantically important.

---

## 3. Commands

Commands represent intent.

Examples:

```text
CreateConversation
PostUserInput
InvokeModel
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

Agent Run
    the runtime is attempting to make something happen

Conversation Event
    something happened in the conversation
```

For simple operations, handling may be synchronous.

For effectful operations such as model invocation or tool execution, runtime orchestration may emit many events over time.

Do not force all command handling into an abstraction such as:

```text
handle(command) -> Vec<Event>
```

Streaming, failures, persistence, retries, and external I/O make that abstraction too restrictive.

---

## 4. Strongly typed identifiers

All durable entities and cross-event references should use strongly typed identifiers.

Use UUIDv7 internally, wrapped in Rust newtypes.

Conceptually:

```rust
struct ConversationId(Uuid);
struct ConversationEventId(Uuid);

struct AgentRunId(Uuid);
struct AgentRunEventId(Uuid);

struct ToolCallId(Uuid);
```

The compiler should prevent accidental substitution of one identifier type for another.

Serialized forms should include the type where practical:

```text
conversation_019...
conversation_event_019...
agent_run_019...
agent_run_event_019...
tool_call_019...
```

The verbosity is intentional.

Explicitly typed identifiers make logs and prompts less ambiguous for both humans and models and reduce accidental or guessed cross-references.

UUIDv7 provides convenient approximate temporal locality, but UUID ordering is **not** the authoritative event ordering mechanism.

---

## 5. Event positions and replay order

Identity and ordering solve different problems.

Every durable event stream should assign a monotonically increasing position.

Conceptually:

```rust
struct StoredEvent<T> {
    position: u64,
    id: EventId,
    timestamp: DateTime<Utc>,
    schema_version: u32,
    event: T,
}
```

Properties:

- `id`
  - stable identity

- `position`
  - authoritative ordering for deterministic replay

- `timestamp`
  - observed wall-clock time
  - not an ordering authority

- `schema_version`
  - supports persisted schema evolution

Phase 1 may assume a single writer and use a trivial position allocator.

For example:

```text
position = previous_position + 1
```

We do **not** need locks, distributed sequencing, or compare-and-append yet.

The important Phase 1 invariant is:

> Persistence preserves an atomic, monotonically increasing position within each event stream, and replay occurs in position order.

Future implementations may use database sequences, optimistic append, transactions, or another mechanism without changing the domain model.

---

## 6. Semantic relationships versus event order

Ordering and semantic relationships should not be conflated.

For example:

```text
ToolResponse
    tool_call_id: ToolCallId
```

describes a semantic relationship.

Its event `position` describes when that event entered the log.

Use domain-specific identifiers for relationships:

```text
tool_call_id
agent_run_id
source_run_id
```

Do not use event positions as semantic identifiers.

Phase 1 does not require a generic causal graph or arbitrary `previous_event_id` relationships.

If branching or explicit causality later becomes important, that can be added without changing the fundamental replay ordering model.

---

## 7. User

Represents user-provided input.

Do not make the domain model intrinsically text-only.

Prefer content that can eventually support:

```text
text
image
file
audio
structured input
```

Phase 1 only needs text.

---

## 8. Assistant

Represents semantic assistant output added to the conversation.

An `Assistant` event is not equivalent to everything emitted by a model.

A model invocation might emit:

```text
reasoning
text deltas
tool-call deltas
usage
lifecycle events
```

The `Assistant` event is the semantic conversational result derived from those lower-level events.

Example:

```text
AgentRunEvents

    response.created
    text.delta "Hel"
    text.delta "lo"
    output.done

        ↓

ConversationEvent

    Assistant("Hello")
```

---

## 9. ToolCall

Records that a tool invocation was requested.

Example:

```text
ToolCall
    id: tool_call_...
    name: shell
    arguments: ...
    source_run_id: agent_run_...
```

A `ToolCall` is a conversation fact.

It is not itself the imperative operation that executes the tool.

The runtime may derive:

```text
ExecuteTool(tool_call_id)
```

from a pending `ToolCall`.

Every tool call must have a stable typed ID.

---

## 10. ToolResponse

Records the result of executing a tool.

Example:

```text
ToolResponse
    tool_call_id: tool_call_...
    result: ...
```

The relationship between `ToolCall` and `ToolResponse` is explicit through the stable `ToolCallId`.

A missing response can indicate pending runtime work, but Phase 1 does not claim exactly-once tool execution.

Where a tool/API supports idempotency keys, the stable tool call ID should eventually be suitable for use as one.

More sophisticated tool-attempt journaling can be added later.

---

## 11. Context

Represents state that affects subsequent model execution.

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

The semantic distinction is:

```text
Context
    potentially contributes to future model input

Data
    metadata about the conversation
    not model input by default
```

---

## 12. Automation

Represents an external or asynchronous actor contributing something to the conversation.

This is distinct from a tool response.

For example:

```text
ToolResponse
    response to a model-requested tool

Automation
    independent external process posts information
```

This distinction should remain explicit.

---

## 13. Data

Represents durable machine-readable metadata associated with the conversation.

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

# Agent Runs

## 14. AgentRun

An `AgentRun` represents a durable runtime attempt to advance a conversation.

Conceptually:

```rust
struct AgentRun {
    id: AgentRunId,
    conversation_id: ConversationId,
    created_at: DateTime<Utc>,
}
```

The exact persisted representation can evolve.

The important distinction is:

```text
InvokeModel
    transient intent

AgentRun
    durable attempt

AgentRunEvents
    durable history of the attempt

ConversationEvents
    semantic consequences
```

Phase 1 may initially create one `AgentRun` per model invocation.

Later, an agent run may encompass multiple model and tool operations if that proves more useful.

Do not over-generalize this yet.

---

## 15. AgentRunEvent

An agent run has its own durable append-only event stream.

Conceptually:

```text
AgentRunEvent
    RunStarted
    ModelProviderEvent
    RunCompleted
    RunFailed
```

This is intentionally small in Phase 1.

Later it may include richer operational events such as:

```text
ToolStarted
ToolCompleted
RetryStarted
RetryCompleted
Checkpoint
```

but these are not required now.

---

## 16. Agent-run event bus boundary

The architecture should behave as though `AgentRunEvent`s are emitted onto an event stream or bus.

Phase 1 does **not** require actual messaging infrastructure.

A synchronous/local sink is sufficient.

Conceptually:

```rust
trait AgentRunEventSink {
    fn emit(&mut self, event: AgentRunEvent) -> Result<()>;
}
```

The OpenAI adapter may therefore operate approximately as:

```text
run started
    ↓
call OpenAI
    ↓
receive provider event
    ↓
persist AgentRunEvent
    ↓
project it
    ↓
receive next provider event
```

The seam matters now.

The infrastructure behind the seam can remain trivial.

---

## 17. Raw model/provider events

Raw provider events belong in the Agent Run Log, not the Conversation Log.

For OpenAI Responses this may include events such as:

```text
response.created
output_item.added
reasoning-related events
output_text.delta
function_call_arguments.delta
response.completed
...
```

Do not immediately collapse these into chat messages.

Persist them as open provider events.

Conceptually:

```rust
struct ModelProviderEvent {
    provider: ProviderId,
    model: String,
    event_type: String,
    payload: Value,
}
```

wrapped in:

```text
AgentRunEvent
    ModelProviderEvent(...)
```

Avoid defining a comprehensive provider-neutral enum of every possible model event.

---

## 18. Preserve provider payloads

Preserve the provider event payload as faithfully as practical.

Where feasible, persistence should occur from the original provider/SSE representation rather than exclusively from a normalized SDK object.

SDKs may:

```text
discard unknown fields
normalize structures
rename values
fail to expose newly introduced fields
```

We want retained provider history to remain useful as APIs evolve.

"Raw" does not require byte-for-byte archival in Phase 1.

It means:

> avoid unnecessary information loss.

No OpenAI SDK-specific type should cross the OpenAI provider boundary.

---

## 19. Agent run lifecycle durability

A durable `AgentRun` must exist before the external model call begins.

This closes the crash window where the runtime sends a request but receives no provider event before terminating.

Conceptually:

```text
InvokeModel
    ↓

create AgentRun
    id = agent_run_...

    ↓

append RunStarted

    ↓

call OpenAI
```

The run identity exists before external I/O.

The run should retain enough request/configuration information to understand what was attempted.

Phase 1 should capture at least the essential invocation configuration, such as:

```text
provider
model
relevant instructions/configuration
tool definitions or references
input/replay strategy
```

The precise snapshot format may evolve.

---

## 20. Provider response identity

Provider-side execution identifiers should be captured when available.

For OpenAI, this includes response IDs.

Example:

```text
AgentRun
    id: agent_run_...

AgentRunEvent
    provider response created
    response_id: resp_...
```

Local `AgentRunId` remains authoritative for our system.

Provider IDs are external references.

---

# Projections

## 21. Projection philosophy

The main abstraction over provider events is not a universal taxonomy of model events.

Instead, we care about what can be **projected** from stored history.

Three projections matter initially:

```text
provider/model replay
conversation semantics
runtime/control
```

The CLI is then a projection of the semantic conversation.

---

## 22. Native/provider replay projection

Agent-run history may contain provider-specific state useful for subsequent invocations.

For OpenAI, this may include:

```text
response IDs
reasoning state/items
completed output items
tool-call state
```

The OpenAI provider owns the logic that decides what is needed for another OpenAI invocation.

This is not a simple per-event boolean such as:

```text
send_to_model = true
```

Projection may:

```text
combine events
discard deltas
retain completed items
retain provider state
construct tool results
```

The provider implementation may inspect an entire run or conversation history to produce new provider input.

---

## 23. Provider continuation versus local reconstruction

Provider-side continuation and local reconstruction are distinct mechanisms.

### Provider-side continuation

Example:

```text
previous_response_id
```

This may allow OpenAI to continue from provider-retained state.

It depends on provider behavior, retention, and availability.

### Local reconstruction

The runtime constructs a new request from locally persisted history.

This may involve:

```text
ConversationEvents
AgentRunEvents
stored request/configuration
semantic tool history
```

Provider-side continuation is useful but should not be the only durable replay mechanism.

Phase 1 may use provider continuation where convenient.

The architecture should not assume that provider-retained state is permanent or portable.

---

## 24. Same-provider replay

When continuing with the same provider and sufficient native history is available:

```text
Conversation
+
Agent Run history
    ↓
OpenAI-native projection
    ↓
OpenAI request
```

This allows higher-fidelity continuation and may retain provider-specific state.

When native model history is authoritative, derived semantic events must not also be blindly replayed as duplicate model input.

For example:

```text
raw OpenAI output events
+
derived Assistant event
```

represent the same semantic output.

The OpenAI projector must choose one authoritative representation.

---

## 25. Cross-provider replay

A different provider cannot necessarily understand native OpenAI events.

Therefore cross-provider replay uses the semantic Conversation Log.

Conceptually:

```text
ConversationEvent
    User
    Assistant
    ToolCall
    ToolResponse
    Context
```

becomes provider-neutral input which the new provider adapter translates into its own format.

Thus:

```text
same provider
    native/high-fidelity replay when possible

different provider
    semantic conversation replay
```

Phase 1 only needs OpenAI, but this distinction should remain in the architecture.

---

## 26. Semantic conversation projection

Agent-run events may derive zero or more semantic conversation events.

Examples:

```text
OpenAI response.created
    → nothing

OpenAI reasoning delta
    → no ConversationEvent

OpenAI completed function call
    → ToolCall

OpenAI completed assistant output
    → Assistant

OpenAI usage
    → possibly Data
```

Derived semantic events should reference their source run where useful.

Phase 1 should also retain the exact `AgentRunEventId` that caused a semantic projection where there is a natural terminal/source event.

For example:

```text
Assistant
    source_run_id: AgentRunId
    source_run_event_id: AgentRunEventId

ToolCall
    source_run_id: AgentRunId
    source_run_event_id: AgentRunEventId
```

A semantic event may be derived from several low-level deltas, but should use a stable source event for projection identity, typically the terminal/provider event that makes the semantic result complete.

---

## 27. Cross-log projection consistency

The Conversation Log and Agent Run Log are persisted independently.

A process may therefore crash after a source `AgentRunEvent` is durable but before its derived semantic `ConversationEvent` is written.

Correctness must not depend on an atomic transaction across the two logs.

Instead:

> Semantic projection from Agent Run history must be reproducible and idempotent.

Rules:

1. Derived `ConversationEvent`s carry stable provenance identifying the source agent run and source run event that caused the projection.
2. Re-projecting the same durable Agent Run history must not create duplicate semantic events.
3. Projection identity combines the source run event with a stable semantic discriminator, because one source event may produce more than one `ConversationEvent`.
4. Persistence should reject or ignore a second semantic projection with the same projection identity.
5. Conversation events created directly from commands or external input, such as `User`, do not require agent-run provenance.
6. On restart or recovery, Agent Run history may be reprojected to recover missing semantic events.

Conceptually:

```text
ProjectionIdentity
    source_run_event_id
    conversation_event_kind
    output_index
```

The exact discriminator may instead use a stable provider output-item ID when one is available.

Example crash:

```text
AgentRunEvent
    output.done
    id = agent_run_event_X

persisted

CRASH

Assistant not persisted
```

Recovery:

```text
reload AgentRunEvents
    ↓
re-run semantic projector
    ↓
output.done derives Assistant
    ↓
no existing Assistant sourced from agent_run_event_X
    ↓
append Assistant
```

If the `Assistant` had already been persisted:

```text
re-run semantic projector
    ↓
matching semantic projection already exists
    ↓
skip
```

Phase 1 does not require distributed transactions between the two logs.

Durable source events plus idempotent projection provide the recovery model.

---

## 28. Runtime/control projection

Runtime logic may derive pending work from durable state.

For example:

```text
ToolCall(tool_call_42)
ToolResponse(tool_call_42) absent
    ↓
ExecuteTool(tool_call_42)
```

while:

```text
ToolCall(tool_call_42)
ToolResponse(tool_call_42) present
    ↓
no action
```

This projection operates over history, not individual isolated events.

Phase 1 only needs enough runtime projection to support basic tool-call round-tripping.

It does not need a generalized workflow engine.

---

# CLI

## 29. CLI output is a Conversation projection

The normal CLI surface should be derived from the semantic Conversation Log.

It should not directly expose raw provider streaming events.

Conceptually:

```text
AgentRunEvents
    ↓
semantic projection
    ↓
ConversationEvents
    ↓
CLI projection
```

Example:

```text
AgentRunEvent
    text.delta "Hel"

AgentRunEvent
    text.delta "lo"

AgentRunEvent
    output.done

        ↓

ConversationEvent
    Assistant("Hello")

        ↓

CLI
    Hello
```

This preserves the CLI contract that stdout represents the semantic/final conversation result rather than raw model transport activity.

---

## 30. Interactive progress

A future interactive renderer may choose to combine:

```text
ConversationEvents
+
selected AgentRunEvents
```

to show:

```text
streaming text
reasoning summaries
tool progress
status
```

That is a separate UI projection.

It should not redefine the canonical CLI output semantics.

Phase 1 does not need to solve this.

---

# OpenAI Phase 1

## 31. Provider abstraction

Define a narrow provider boundary around actual needs.

Conceptually:

```text
Conversation
+
AgentRun history
    ↓

provider input projection
    ↓

ModelRequest
    ↓

OpenAI provider
    ↓

AgentRunEvent stream
```

The OpenAI implementation owns:

```text
Responses API request construction
HTTP/SSE behavior
OpenAI response parsing
provider event persistence representation
native replay
provider continuation
reasoning/state handling
```

No OpenAI-specific SDK/API type crosses the provider boundary.

---

## 32. OpenAI implementation strategy

Phase 1 will implement directly against the OpenAI Responses API.

The purpose is partly architectural discovery.

We want firsthand understanding of the newer model/event shape before adopting a multi-provider abstraction.

This does not make OpenAI our domain model.

The OpenAI implementation is an adapter.

A later implementation may use:

```text
rust-genai
another provider library
direct Anthropic integration
direct Gemini integration
```

behind the same boundaries.

---

## 33. Phase 1 OpenAI scope

Support enough to exercise the architecture:

```text
basic text input
Responses API invocation
streaming SSE events
capture provider events
response IDs
usage
reasoning/state where available
function/tool calls
function/tool responses
continuation/reinvocation
```

Do not attempt complete OpenAI Responses coverage.

The following are out of scope unless nearly free:

```text
hosted web search
file search
computer use
image generation
background execution
```

---

## 34. Incremental persistence

Agent-run events should be persisted incrementally as they arrive.

Conceptually:

```text
receive provider event
    ↓
assign AgentRunEventId
assign run position
    ↓
persist
    ↓
project
    ↓
continue stream
```

Do not wait until the complete model response before recording operational history.

This gives:

```text
debuggability
partial crash history
deterministic replay
visibility into provider behavior
```

---

# Security

## 35. Phase 1 security baseline

Raw model events, context, and tool output may contain sensitive information.

Examples include:

```text
credentials
tokens
environment variables
private file contents
provider metadata
tool output
user data
```

Phase 1 does not need a complete retention/redaction framework.

However, it should not knowingly persist obvious credentials.

At minimum:

- exclude known secrets where identifiable
- avoid intentionally capturing environment secrets
- persist conversation/run data with private filesystem permissions
- document that raw event persistence may contain sensitive data

A complete policy for retention, permissions, and redaction belongs in a later phase.

---

# Phase 1 philosophy

## 36. What we are committing to

Phase 1 commits to:

```text
ConversationEvents as semantic history

AgentRunEvents as operational/provider history

strongly typed UUIDv7 IDs

monotonic per-stream positions for deterministic replay

provider-specific raw payload preservation

clear provider boundaries

semantic versus native replay distinction

idempotent projection between Agent Run and Conversation logs

CLI as a projection of the Conversation Log
```

These are architectural seams we do not want to need to undo.

---

## 37. What we are intentionally not solving

Phase 1 does not require:

```text
distributed event buses
locks or concurrent append coordination
global event clocks
generic causal DAGs
cross-log distributed transactions
exactly-once tool execution
universal provider event taxonomy
perfect cross-provider replay
general workflow orchestration
complete failure taxonomy
production-grade security policy
complete OpenAI Responses coverage
```

Where uncertain:

> Preserve information and keep the boundary open.

---

## 38. First implementation milestone

The first coherent implementation should prove:

```text
CreateConversation

PostUserInput("hello")

ConversationEvent
    User("hello")

InvokeModel

create AgentRun

AgentRunEvent
    RunStarted

call OpenAI Responses API

receive provider events

persist each as AgentRunEvent
with typed IDs and monotonic positions

derive semantic Assistant event
with stable source AgentRunEventId

persist ConversationEvent
    Assistant(...)

AgentRunEvent
    RunCompleted

reload persisted state

re-run semantic projection safely
without duplicate ConversationEvents

project Conversation to CLI

reinvoke OpenAI using the appropriate replay strategy
```

Then add:

```text
tool call
tool execution
tool response
model reinvocation
```

and validate the boundaries rather than attempting full runtime sophistication.

---

## 39. Documentation status

This document describes a **Phase 1 architectural direction**, not a finished permanent API.

Implementation experience should feed back into it.

If OpenAI Responses exposes assumptions that conflict with this model, document those pressures explicitly rather than hiding them behind increasingly elaborate abstractions.

Related architecture documents should be updated so they do not continue to describe these decisions as unresolved.

Add this document to the documentation index.

Major future architectural changes should be captured in ADRs when useful.
