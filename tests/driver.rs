// ban than dong ...

mod support;

use std::path::PathBuf;

use support::{compiler, invoke, missing_runtime_library_message, project_root, runtime_library};

fn stdout_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

// ===== xu ly doi so =====

#[test]
fn version_prints_the_package_version() {
    let output = invoke(&["--version"]);
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(
        stdout_of(&output).trim(),
        format!("pump {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn help_describes_both_commands() {
    let output = invoke(&["--help"]);
    assert!(output.status.success(), "{}", stderr_of(&output));
    let text = stdout_of(&output);
    assert!(text.contains("pump run"), "{text}");
    assert!(text.contains("pump build"), "{text}");
}

#[test]
fn no_arguments_prints_the_usage() {
    let output = invoke(&[]);
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert!(stdout_of(&output).contains("usage:"));
}
