use std::ffi::OsString;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::conversation::{ConversationId, ModelEventImportance, ModelId};
use crate::openai::OpenAiModelDriver;
use crate::persistence::EventStore;
use crate::turn::{TurnProgress, TurnRequest, TurnResult, TurnResultValue, TurnService};

#[derive(Debug, Parser)]
#[command(
    name = "tog",
    version,
    about = "Command-line access to agentic services",
    disable_help_subcommand = true,
    override_usage = "tog [:turn] [OPTIONS] <USER_PROMPT>...",
    after_help = "When no command is specified, :turn is used."
)]
pub(crate) struct CommandLine {
    #[command(subcommand)]
    command: Command,
}

impl CommandLine {
    pub(crate) fn parse_with_default_command() -> Self {
        let mut arguments: Vec<OsString> = std::env::args_os().collect();
        let first_argument = arguments.get(1).and_then(|argument| argument.to_str());
        let has_command_or_root_option = first_argument.is_some_and(|argument| {
            argument.starts_with(':') || matches!(argument, "--help" | "-h" | "--version" | "-V")
        });
        if !has_command_or_root_option {
            arguments.insert(1, OsString::from(":turn"));
        }
        Self::parse_from(arguments)
    }

    pub(crate) fn execute(self) -> TurnResultValue<TurnResult> {
        match self.command {
            Command::Turn(arguments) => {
                let user_prompt = arguments.user_prompt_words.join(" ").parse()?;
                let verbosity = arguments.verbosity;
                let turn_service = TurnService::new(
                    EventStore::from_environment()?,
                    Box::new(OpenAiModelDriver::from_environment(arguments.model)?),
                );
                let mut turn_result = turn_service.execute(
                    TurnRequest {
                        conversation_id: arguments.conversation,
                        user_prompt,
                    },
                    |conversation_id| eprintln!("#> conversation {conversation_id}"),
                    |progress| match progress {
                        TurnProgress::ModelInvocationStarted { model } => {
                            eprintln!("## waiting for model {model}");
                        }
                    },
                )?;
                turn_result
                    .model_events
                    .retain(|event| verbosity.includes(event.importance()));
                Ok(turn_result)
            }
        }
    }
}

#[derive(Debug, Subcommand)]
enum Command {
    #[command(
        name = ":turn",
        override_usage = "tog [:turn] [OPTIONS] <USER_PROMPT>..."
    )]
    Turn(TurnArguments),
}

#[derive(Debug, Args)]
struct TurnArguments {
    #[arg(long)]
    conversation: Option<ConversationId>,

    #[arg(long, default_value = "gpt-5.6")]
    model: ModelId,

    #[arg(long, value_enum, default_value = "low")]
    verbosity: Verbosity,

    #[arg(value_name = "USER_PROMPT", num_args = 1.., required = true)]
    user_prompt_words: Vec<String>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Verbosity {
    Low,
    Medium,
    High,
}

impl Verbosity {
    fn includes(self, importance: ModelEventImportance) -> bool {
        match self {
            Self::Low => importance >= ModelEventImportance::Important,
            Self::Medium => importance >= ModelEventImportance::Interesting,
            Self::High => true,
        }
    }
}
