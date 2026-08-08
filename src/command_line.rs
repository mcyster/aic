use clap::{Args, Parser, Subcommand};

use crate::conversation::{AssistantResponse, StubConversationService, UserPrompt};

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
    pub(crate) fn execute(self) -> AssistantResponse {
        match self.command {
            Command::Turn(arguments) => StubConversationService.respond_to(arguments.user_prompt),
        }
    }
}

#[derive(Debug, Subcommand)]
enum Command {
    Turn(TurnArguments),
}

#[derive(Debug, Args)]
struct TurnArguments {
    user_prompt: UserPrompt,
}
