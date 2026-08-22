# 0003: Conversation Is the Portable Model Input

## Status

Accepted

## Context

ADR 0002 established that a `ModelDriver` receives an immutable view of the semantic Conversation Log and that continuation must not require provider-native history. That boundary is portable only if the conversation contains or immutably references all semantic state made visible to the model.

Allowing `ModelDriver::invoke` to accept a separate bag of instructions, tool definitions, context, or other model-visible inputs would create unrecorded state. A later invocation could not reliably reconstruct what the model saw, and another compatible driver could not continue the conversation without mutable state or ambient configuration belonging to the original driver.

## Decision

Keep `ModelDriver::invoke` centered on the immutable conversation. Everything made visible to a model must be recorded in the conversation or immutably referenced by it, including user and model content, instructions, context, model-visible tool definitions, tool requests and responses, and referenced files or images.

Provider transport configuration, credentials, executable tool implementations, retry policy, and orchestration mechanics remain outside the conversation. Future model-visible capabilities must become conversation concepts or immutable references rather than ambient driver configuration.

A compatible `ModelDriver` must be able to construct its provider request and continue the conversation without provider-native history or mutable state owned by another driver.

## Consequences

The Conversation Log is the portable input for semantic continuation and replay across providers. Provider-native continuation may optimize fidelity, latency, or cost, but cannot be required for correctness. Replay does not promise deterministic reproduction of model output.

Making the conversation the portable model input makes the semantic conversation domain conversation-centered and event-sourced. The append-only Conversation Log is its authoritative state: current views are derived by replay, and new semantic state is introduced only by appending durable semantic facts. This does not make commands, transport events, raw deltas, retries, diagnostics, execution mechanics, or the rest of `tog` event-sourced. Those operational details may use tracing or a separate log without entering the canonical conversation.

Portability requires conversation events to define a universal semantic minimum, while fidelity permits lossless enrichment beyond that minimum. Every compatible driver must understand the minimum, may interpret recognized enrichment, and must safely ignore enrichment it does not understand. Enrichment may support richer or more efficient continuation, but it does not replace portable fields. Important structured semantics such as a tool call ID, tool name, and arguments remain portable. Extensions should identify their owning driver or namespace and schema version when needed for unambiguous interpretation; raw provider transport events do not become semantic facts merely by being provider-specific.

Exact reconstruction of a historical provider request may require durable invocation metadata in addition to the conversation, including the provider, model, parameters, and projection version. That metadata records how the portable semantic input was projected; it does not replace the conversation or permit unrecorded model-visible semantic input.
