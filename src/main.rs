mod command_line;
mod conversation;
mod model_driver;
mod openai;
mod persistence;
mod turn;

use std::error::Error;

use command_line::CommandLine;
use conversation::ModelEvent;

fn main() -> Result<(), Box<dyn Error>> {
    let command_line = CommandLine::parse_with_default_command();
    let turn_result = command_line.execute()?;

    for model_event in turn_result.model_events {
        let prefix = match &model_event {
            ModelEvent::AssistantResponse(_) => "",
            ModelEvent::Communication(_) => "### ",
        };
        println!("{prefix}{}", model_event.message());
    }
    Ok(())
}
