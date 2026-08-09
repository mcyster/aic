use clap::{Args, Parser, Subcommand};

use crate::agent_run::ResponseVerbosity;
use crate::conversation::UserPrompt;
use crate::identifier::ConversationId;
use crate::turn::{TurnProgress, TurnRequest, TurnResult, TurnResultValue, TurnService};

#[derive(Debug, Parser)]
#[command(
    name = "aic",
    version,
    about = "Command-line access to agentic services"
)]
pub(crate) struct CommandLine {
    #[command(subcommand)]
    command: Command,
}

impl CommandLine {
    pub(crate) fn execute(self) -> TurnResultValue<TurnResult> {
        match self.command {
            Command::Turn(arguments) => TurnService::from_environment()?.execute(
                TurnRequest {
                    conversation_id: arguments.conversation,
                    model: arguments.model,
                    response_verbosity: arguments.response_verbosity,
                    user_prompt: arguments.user_prompt,
                },
                |conversation_id| eprintln!("#> conversation {conversation_id}"),
                |progress| match progress {
                    TurnProgress::ModelInvocationStarted { model } => {
                        eprintln!("## waiting for OpenAI model {model}");
                    }
                    TurnProgress::ProviderEventsReceived { count } => {
                        let event_label = if count == 1 { "event" } else { "events" };
                        eprintln!("## receiving OpenAI response ({count} provider {event_label})");
                    }
                },
            ),
        }
    }
}

#[derive(Debug, Subcommand)]
enum Command {
    Turn(TurnArguments),
}

#[derive(Debug, Args)]
struct TurnArguments {
    #[arg(long)]
    conversation: Option<ConversationId>,

    #[arg(long, default_value = "gpt-5.6")]
    model: String,

    #[arg(long = "verbosity", default_value = "low")]
    response_verbosity: ResponseVerbosity,

    user_prompt: UserPrompt,
}
