# Architecture

`aic` is a single Cargo package containing one executable.

The command-line module translates process arguments into typed commands. The conversation module owns conversation-specific values and behavior. The executable composes these modules and writes the response to standard output.

The initial conversation service is deterministic and does not perform network access. A provider will be selected before introducing asynchronous execution, HTTP dependencies, provider configuration, or provider abstractions.
