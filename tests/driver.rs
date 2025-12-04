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

#[test]
fn every_shipped_example_runs() {
    let directory = project_root().join("examples");
    let mut examples: Vec<PathBuf> = std::fs::read_dir(&directory)
        .unwrap_or_else(|error| panic!("cannot read `{}`: {error}", directory.display()))
        .map(|entry| entry.expect("a readable directory entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "pump")
        })
        .collect();
    examples.sort();

    assert!(!examples.is_empty(), "there are no examples to run");

    let mut broken = Vec::new();
    for example in &examples {
        let output = invoke(&["run", &example.display().to_string()]);
        if !output.status.success() {
            broken.push(format!(
                "  {}\n{}",
                example.display(),
                stderr_of(&output).trim_end()
            ));
        }
    }

    assert!(
        broken.is_empty(),
        "these examples no longer run:\n{}",
        broken.join("\n")
    );
}

// ===== `pump build` =====

#[test]
fn build_produces_a_standalone_executable() {
    assert!(
        runtime_library().is_some(),
        "{}",
        missing_runtime_library_message()
    );

    let source = project_root()
        .join("tests")
        .join("cases")
        .join("run")
        .join("hello_world.pump");
    let output_path = std::env::temp_dir().join("pump_build_hello_world.exe");
    let _ = std::fs::remove_file(&output_path);

    let built = invoke(&[
        "build",
        &source.display().to_string(),
        "-o",
        &output_path.display().to_string(),
    ]);
    assert!(
        built.status.success(),
        "`pump build` failed:\n{}",
        stderr_of(&built)
    );
    assert!(
        output_path.is_file(),
        "`pump build` reported success but wrote no file at `{}`",
        output_path.display()
    );

    let executed = std::process::Command::new(&output_path)
        .output()
        .unwrap_or_else(|error| panic!("cannot run `{}`: {error}", output_path.display()));
    assert!(
        executed.status.success(),
        "the built executable exited with {}:\n{}",
        executed.status,
        String::from_utf8_lossy(&executed.stderr)
    );

    let jitted = invoke(&["run", &source.display().to_string()]);
    assert_eq!(
        String::from_utf8_lossy(&executed.stdout),
        String::from_utf8_lossy(&jitted.stdout),
        "`pump build` and `pump run` disagree about the program's output"
    );

    let _ = std::fs::remove_file(&output_path);
}

#[test]
fn a_built_executable_panics_the_same_way() {
    assert!(
        runtime_library().is_some(),
        "{}",
        missing_runtime_library_message()
    );

    let source = project_root()
        .join("tests")
        .join("cases")
        .join("panic")
        .join("explicit_panic.pump");
    let output_path = std::env::temp_dir().join("pump_build_explicit_panic.exe");
    let _ = std::fs::remove_file(&output_path);

    let built = invoke(&[
        "build",
        &source.display().to_string(),
        "-o",
        &output_path.display().to_string(),
    ]);
    assert!(
        built.status.success(),
        "`pump build` failed:\n{}",
        stderr_of(&built)
    );

    let executed = std::process::Command::new(&output_path)
        .output()
        .unwrap_or_else(|error| panic!("cannot run `{}`: {error}", output_path.display()));
    assert_eq!(executed.status.code(), Some(101));
    assert_eq!(String::from_utf8_lossy(&executed.stdout).trim(), "before");
    assert!(
        String::from_utf8_lossy(&executed.stderr).contains("pump: panic: stop here"),
        "{}",
        String::from_utf8_lossy(&executed.stderr)
    );

    let _ = std::fs::remove_file(&output_path);
}

#[test]
fn build_writes_nothing_when_compilation_fails() {
    let source = project_root()
        .join("tests")
        .join("cases")
        .join("fail")
        .join("type_mismatch.pump");
    let output_path = std::env::temp_dir().join("pump_build_type_mismatch.exe");
    let _ = std::fs::remove_file(&output_path);

    let built = invoke(&[
        "build",
        &source.display().to_string(),
        "-o",
        &output_path.display().to_string(),
    ]);
    assert!(!built.status.success());
    assert!(stderr_of(&built).contains("error[E0550]"));
    assert!(
        !output_path.exists(),
        "a failed build left `{}` behind",
        output_path.display()
    );
}

#[test]
fn the_compiler_binary_exists_where_cargo_says() {
    assert!(
        compiler().is_file(),
        "`{}` is not a file",
        compiler().display()
    );
}
