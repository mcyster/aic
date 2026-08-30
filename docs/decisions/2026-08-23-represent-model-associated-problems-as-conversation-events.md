# Represent Model-Associated Problems as Conversation Events

## Why

Model limitations and invocation failures both need a portable, sanitized durable representation. They remain distinct from detailed Rust control-flow errors, but both are facts associated with a selected model invocation and therefore have a meaningful `ModelSource`.

Introducing a broad conversation-problem hierarchy or separate top-level event variants would predict requirements for tool, filesystem, and orchestration failures that do not yet exist. A trait hierarchy would also duplicate the closed enum hierarchy needed for serialization.

## Decision

Use `ConversationEventKind::Problem { source, problem }` as the one durable problem surface. `ModelProblem::Issue(ModelIssue)` represents a meaningful limitation or unsuccessful model outcome, and `ModelProblem::Invocation(InvocationError)` represents a sanitized invocation failure. `ConversationEventKind::Model` remains limited to successful semantic `ModelEvent` output.

`ModelProblem` is a closed serializable enum. Every concrete variant provides one meaningful sanitized message. The parent delegates common `message` and `retryable` behavior to its categories without introducing a trait. The conversation event does not duplicate the message and does not add generic severity or impact fields.

A concrete driver yields `ModelDriverOutput::Issue` when it can translate provider-specific information into portable semantics. The turn service wraps it as `ModelProblem::Issue`. `ModelDriverError` remains the detailed error returned by the invocation future or stream. The turn service converts that error into a sanitized invocation problem, appends it under the invoked driver's source, and then returns the original error for Rust control flow.

Durable problem messages must not contain credentials, authorization headers, raw provider bodies, stack traces, sensitive request data, or diagnostics without portable conversational meaning. Detailed diagnostics may remain in `ModelDriverError` and application logging.

## Consequences

Later orchestration can observe preceding failures and limitations from immutable conversation history. Provider projection decides whether and how to expose a problem to a later model; retaining it canonically does not require verbatim replay in every provider request.

The current problem hierarchy remains model-specific. Future tool, filesystem, or orchestration problems will be designed from their concrete provenance and behavior rather than being forced into this model-associated event.
