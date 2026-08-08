mod command_line;
mod conversation;

use clap::Parser;
use command_line::CommandLine;

fn main() {
    let command_line = CommandLine::parse();
    let assistant_response = command_line.execute();

    println!("{assistant_response}");
}
