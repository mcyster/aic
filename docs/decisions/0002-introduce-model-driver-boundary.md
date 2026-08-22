# 0002: Introduce the ModelDriver Boundary

## Status

Accepted

## Context

The first provider integration established concrete OpenAI Responses transport and replay concerns. Allowing those concepts to remain in turn orchestration would make provider-native history a requirement for conversation continuation and would prevent model or provider switching from semantic history alone.

This is the concrete requirement anticipated by ADR 0001's restriction on speculative provider abstractions.

## Decision

Introduce a narrow `ModelDriver` boundary. A configured driver exposes its typed provider/model source, receives an immutable view of the semantic Conversation Log, and produces zero or more typed `ModelEvent`s or a typed model error. The caller records the driver's source when it converts produced model events into canonical `ConversationEvent`s. ADR 0004 supersedes this decision's original batch delivery and failure semantics with asynchronous streaming delivery.

OpenAI request types, streaming events, response IDs, and transport errors remain inside the OpenAI implementation. The Conversation Log is the only durable history required for correctness. Provider-native continuation may be added later only as an optional optimization.

## Consequences

Turn orchestration and the CLI do not depend on OpenAI protocol concepts. Every invocation currently reconstructs provider input from semantic history, which costs more input tokens than provider-native continuation but proves cross-driver portability. The abstraction remains intentionally small and may change when a second production driver provides more evidence. ADR 0004 defines incremental persistence and late stream failure behavior.
