---
name: project-documentation
description: Write or revise concise project documentation under docs/, including plans, notes, and decisions. Use when preserving project intent, investigations, architectural reasoning, or intended work; do not use for end-user manuals or generated API reference.
---

# Project Documentation

Write documentation for people working on the project, including people returning
after the original context has been forgotten. A document is not successful merely
because an LLM can summarize it.

## Documentation structure

- `docs/` contains concise documentation of the project: its purpose, important
  concepts, constraints, and intent.
- `docs/plans/` contains intended work. A plan describes what should change and the
  important choices involved; it is not evidence that the work has been completed.
- `docs/notes/` contains developing thoughts, brainstorming, discussions,
  investigations, experiments, and useful conclusions from AI conversations. Notes
  are working history: they may be incomplete, superseded, or wrong, and are not
  authoritative.
- `docs/decisions/` contains the small current set of significant decisions and why
  they were made. Decisions should be shorter and easier to scan than notes.

Add more specific areas such as `docs/designs/` or `docs/architecture/` only when
the project has enough durable material to justify them. Do not create empty
taxonomies in anticipation of future documents.

## File names

Use short, specific, lowercase names separated by hyphens.

- Notes capture thinking at a moment, so name them
  `docs/notes/YYYY-MM-DD-short-specific-name.md`.
- Decisions are events, so name them
  `docs/decisions/YYYY-MM-DD-decision-statement.md`. State the decision rather
  than merely its subject: prefer
  `2026-08-29-use-semantic-conversation-events.md` to
  `2026-08-29-conversation-events.md`.
- Plans are living documents, so omit the date and name them by intended outcome,
  such as `docs/plans/add-tool-execution.md`.
- Designs and architecture describe the system, so omit the date and name them by
  concept, such as `docs/architecture/conversation.md`.

Do not date-prefix everything. Dates distinguish historical notes and decisions
from living descriptions. Use the date the note or decision was first recorded;
ordinary revisions do not rename the file.

Do not add status metadata to notes. Their date and location already communicate
that they are historical working material. Do not maintain a README that lists each
note or decision; browse the directories directly.

## Write for human readers

Lead with the point. Preserve the result of the thinking rather than its chronology.
Remove conversational turns, repeated context, false starts, generic background,
and exhaustive inventories that a reader can obtain more accurately from the code.

Keep the few things that change understanding or implementation:

- the problem or goal;
- relevant facts and constraints;
- important boundaries and invariants;
- alternatives that remain plausible or explain a non-obvious choice;
- critical pivot points where a different choice would produce a different design;
- the current conclusion, intended outcome, or unresolved question.

Use only the headings the subject needs. Prefer a short coherent document over a
large standard template. Clearly distinguish observations, conclusions, current
preferences, and open questions when the distinction matters.

For AI-assisted investigations, synthesize a durable note. Do not save a raw chat
transcript unless the exact exchange is itself important evidence. A human reader
should understand the note without access to the original conversation.

Keep a decision focused on the decision, its essential reasoning, and important
consequences. Link to a note for deeper investigation rather than copying the note
into the decision. Keep only current decisions in `docs/decisions/`. When one is
superseded, replace or remove it in the same change; Git retains the old decision.

## Keep architecture visible

Prefer code that clearly expresses the implemented architecture through its module
structure, types, names, interfaces, dependencies, and enforced boundaries.
Documentation should not duplicate that structure. Use documentation to explain the
intent and reasoning the code cannot express clearly: why boundaries exist, which
constraints matter, what direction is intended, and where the implementation is
known to fall short.

Do not assume that the current code is the intended architecture. When code and
documentation disagree, identify the difference between current implementation and
intended design. Do not silently rewrite the documentation to match accidental code,
or extend the conflicting implementation as though it were authoritative.

When changing architecture, make the code express the new architecture wherever
practical and update the small amount of documentation that carries intent.

## Keep plans and designs focused

A plan or design should help someone make and review the change, not display the
author's reasoning process. Center it on the goal, constraints, boundaries,
important choices, non-goals, and observable completion. Include ordered
implementation detail only where order, risk, or coordination matters. Leave routine
steps to the implementer and the codebase.

If a critical choice is unresolved, state it directly. Do not bury uncertainty in a
long stream of possible implementation details.

## Maintain the documentation

Read relevant code and nearby documents before writing. Update an existing document
instead of creating a competing account when they cover the same subject. Link to
code, issues, plans, notes, or decisions when the link saves meaningful rediscovery.

Treat Git as the history. Notes may preserve historical investigation, while plans,
decisions, and durable project documentation should reflect current intent. When
implementation changes make durable documentation untrue, update the documentation
in the same change.
