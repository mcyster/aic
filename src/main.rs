mod agent_run;
mod command_line;
mod conversation;
mod identifier;
mod openai;
mod persistence;
mod turn;

use std::error::Error;

use clap::Parser;
use command_line::CommandLine;

fn main() -> Result<(), Box<dyn Error>> {
    let command_line = CommandLine::parse();
    let turn_result = command_line.execute()?;

    println!("{}", turn_result.assistant_text);
    Ok(())
}
