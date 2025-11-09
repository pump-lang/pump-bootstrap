// ban than dong lenh `pump`: xu ly doi so, va duong `build` ra file chay.

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

#[test]
fn an_unknown_command_is_rejected() {
    let output = invoke(&["frobnicate", "x.pump"]);
    assert!(!output.status.success());
    let text = stderr_of(&output);
    assert!(text.contains("error[E0704]"), "{text}");
    assert!(text.contains("unknown command `frobnicate`"), "{text}");
}

#[test]
fn run_without_a_file_is_rejected() {
    let output = invoke(&["run"]);
    assert!(!output.status.success());
    let text = stderr_of(&output);
    assert!(text.contains("error[E0704]"), "{text}");
    assert!(text.contains("`pump run` needs a FILE"), "{text}");
}

#[test]
fn output_belongs_to_build_not_run() {
    let output = invoke(&["run", "examples/hello.pump", "-o", "hello.exe"]);
    assert!(!output.status.success());
    let text = stderr_of(&output);
    assert!(text.contains("error[E0704]"), "{text}");
    assert!(
        text.contains("`--output` applies to `pump build`, not `pump run`"),
        "{text}"
    );
}

#[test]
fn a_missing_entry_file_is_reported_with_its_path() {
    let output = invoke(&["run", "tests/cases/run/no_such_case.pump"]);
    assert!(!output.status.success());
    let text = stderr_of(&output);
    assert!(text.contains("error[E0700]"), "{text}");
    assert!(text.contains("no_such_case.pump"), "{text}");
}

// ===== may vi du di kem =====
