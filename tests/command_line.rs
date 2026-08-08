use std::process::Command;

fn aic_command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_aic"))
}

#[test]
fn turn_prints_stub_response_for_user_prompt() {
    let command_output = aic_command()
        .args(["turn", "Explain Rust ownership"])
        .output()
        .expect("aic should run");

    assert!(command_output.status.success());
    assert_eq!(
        String::from_utf8(command_output.stdout).expect("standard output should be UTF-8"),
        "Stub assistant response to: Explain Rust ownership\n"
    );
}

#[test]
fn turn_rejects_a_missing_user_prompt() {
    let command_output = aic_command().arg("turn").output().expect("aic should run");

    assert!(!command_output.status.success());
    assert!(
        String::from_utf8(command_output.stderr)
            .expect("standard error should be UTF-8")
            .contains("required")
    );
}

#[test]
fn help_lists_the_turn_command() {
    let command_output = aic_command()
        .arg("--help")
        .output()
        .expect("aic should run");

    assert!(command_output.status.success());
    assert!(
        String::from_utf8(command_output.stdout)
            .expect("standard output should be UTF-8")
            .contains("turn")
    );
}
