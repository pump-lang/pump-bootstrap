// do dung chung cho ca bo test dau-den-cuoi.
//
// Moi ca la mot chuong trinh .pump nam tren dia canh cai ket qua no phai ra:
//
//   tests/cases/run       NAME.pump        NAME.out, dung y stdout
//   tests/cases/fail      NAME.pump        NAME.err, loi mong doi
//   tests/cases/project   NAME/main.pump   NAME/expected.out
//   tests/cases/os        NAME.pump        NAME.out, nhung co doi so
//
// Bo test goi thang cai binary `pump` nhu mot tien trinh con chu khong goi
// vao thu vien, vi cai ma nguoi dung gap la mot tien trinh: stdout cua no,
// stderr cua no, va ma thoat cua no.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The `pump` executable Cargo built for this test run.
pub fn compiler() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_pump"))
}

/// The package root, which is where the `tests` directory lives.
pub fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// One of the case directories: `run`, `fail` or `project`.
pub fn cases_dir(kind: &str) -> PathBuf {
    project_root().join("tests").join("cases").join(kind)
}

/// Runs `pump` with the given arguments and captures the result.
pub fn invoke(arguments: &[&str]) -> Output {
    let mut command = Command::new(compiler());
    command.args(arguments);
    command.current_dir(project_root());
    // `pump build` phai link voi staticlib cua runtime. Chi thang vao no de
    // viec tim kiem khong phu thuoc thu muc dang dung.
    if let Some(library) = runtime_library() {
        command.env("PUMP_RUNTIME_LIB", library);
    }
    command
        .output()
        .unwrap_or_else(|error| panic!("could not start `{}`: {error}", compiler().display()))
}

/// Locates the runtime static library `pump build` links against.
pub fn runtime_library() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(directory) = compiler().parent() {
        candidates.push(directory.join("pump_runtime.lib"));
        candidates.push(directory.join("deps").join("pump_runtime.lib"));
    }
    let target = project_root().join("target");
    for profile in ["debug", "release"] {
        candidates.push(target.join(profile).join("pump_runtime.lib"));
    }
    candidates.into_iter().find(|path| path.is_file())
}

/// The message a `build` case fails with when the runtime staticlib is
/// absent.
pub fn missing_runtime_library_message() -> String {
    format!(
        "cannot find `pump_runtime.lib`, so `pump build` has nothing to link \
         against.\nBuild the staticlib first:\n\n    cargo build --workspace\n\n\
         A plain `cargo build` does not produce it: the workspace declares no \
         `default-members`, so the `pump-runtime` staticlib is never a \
         requested target.\nLooked beside `{}` and under `{}`.",
        compiler().display(),
        project_root().join("target").display()
    )
}

// ===== ca phai chay duoc =====

/// Compiles and runs `tests/cases/run/NAME.pump`, asserting its stdout
/// matches `tests/cases/run/NAME.out` exactly.
pub fn check_run_case(name: &str) {
    let directory = cases_dir("run");
    let program = directory.join(format!("{name}.pump"));
    let expected = read(&directory.join(format!("{name}.out")));

    let output = invoke(&["run", &program.display().to_string()]);
    if !output.status.success() {
        panic!(
            "{}",
            failure_report(
                name,
                &program,
                "the program was expected to compile and run, but `pump run` failed",
                &output,
            )
        );
    }

    let actual = normalise(&String::from_utf8_lossy(&output.stdout));
    let expected = normalise(&expected);
    if actual != expected {
        panic!(
            "{}",
            diff_report(name, &program, &expected, &actual, &output)
        );
    }
}

/// Compiles and runs `tests/cases/project/NAME/main.pump`, a multi-file case,
/// against `tests/cases/project/NAME/expected.out`.
pub fn check_project_case(name: &str) {
    let directory = cases_dir("project").join(name);
    let program = directory.join("main.pump");
    let expected = read(&directory.join("expected.out"));

    let output = invoke(&["run", &program.display().to_string()]);
    if !output.status.success() {
        panic!(
            "{}",
            failure_report(
                name,
                &program,
                "the project was expected to compile and run, but `pump run` failed",
                &output,
            )
        );
    }

    let actual = normalise(&String::from_utf8_lossy(&output.stdout));
    let expected = normalise(&expected);
    if actual != expected {
        panic!(
            "{}",
            diff_report(name, &program, &expected, &actual, &output)
        );
    }
}

// ===== ca cham vao he dieu hanh =====

/// Compiles and runs `tests/cases/os/NAME.pump` the way `check_run_case`
/// does, but hands the program two arguments: a scratch directory of its own,
/// and the path of the `pump` binary itself.
///
/// Ca o day phai doc ghi file va chay tien trinh con, ma ca hai deu khong the
/// viet cung mot duong dan trong nguon: cai thu muc kia phai la cua rieng lan
/// chay nay. Dua qua argv la cach duy nhat chuong trinh Pump biet no o dau,
/// va tien the thu luon `os.args()`. Con tien trinh con thi chinh la `pump`,
/// nen bo test khong phu thuoc chuong trinh nao co san tren may.
pub fn check_os_case(name: &str) {
    let directory = cases_dir("os");
    let program = directory.join(format!("{name}.pump"));
    let expected = read(&directory.join(format!("{name}.out")));

    let scratch = scratch_dir(name);
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch)
        .unwrap_or_else(|error| panic!("cannot make `{}`: {error}", scratch.display()));

    let output = invoke(&[
        "run",
        &program.display().to_string(),
        "--",
        &scratch.display().to_string(),
        &compiler().display().to_string(),
    ]);
    let _ = std::fs::remove_dir_all(&scratch);

    if !output.status.success() {
        panic!(
            "{}",
            failure_report(
                name,
                &program,
                "the program was expected to compile and run, but `pump run` failed",
                &output,
            )
        );
    }

    let actual = normalise(&String::from_utf8_lossy(&output.stdout));
    let expected = normalise(&expected);
    if actual != expected {
        panic!(
            "{}",
            diff_report(name, &program, &expected, &actual, &output)
        );
    }
}

/// A directory of this case's own, outside the source tree.
fn scratch_dir(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("pump-os-case-{name}-{}", std::process::id()));
    path
}

// ===== ca phai bi tu choi =====

/// What a `.err` file asks of the compiler.
#[derive(Debug, Default)]
pub struct Expectation {
    pub codes: Vec<String>,
    pub texts: Vec<String>,
    pub absent: Vec<String>,
}

/// Reads and validates one `.err` file.
pub fn parse_expectation(path: &Path) -> Expectation {
    let source = read(path);
    let mut expectation = Expectation::default();

    for (index, line) in source.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let value = value.trim().to_string();
        match key {
            "code" => expectation.codes.push(value),
            "text" => expectation.texts.push(value),
            "absent" => expectation.absent.push(value),
            other => panic!(
                "{}:{}: unknown key `{other}`; the keys are `code`, `text` and `absent`",
                path.display(),
                index + 1
            ),
        }
    }

    assert!(
        !expectation.codes.is_empty(),
        "{}: an expectation file must name at least one `code`",
        path.display()
    );
    expectation
}

/// Compiles `tests/cases/fail/NAME.pump`, asserting it is rejected with the
/// diagnostics `tests/cases/fail/NAME.err` describes.
pub fn check_fail_case(name: &str) {
    let directory = cases_dir("fail");
    let program = directory.join(format!("{name}.pump"));
    let expectation = parse_expectation(&directory.join(format!("{name}.err")));

    let output = invoke(&["run", &program.display().to_string()]);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();

    if output.status.success() {
        panic!(
            "case `{name}` ({}) compiled and ran, but it must be rejected with {}.\
             \n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
            program.display(),
            expectation.codes.join(", ")
        );
    }

    let mut problems = Vec::new();
    for code in &expectation.codes {
        let marker = format!("error[{code}]");
        if !stderr.contains(&marker) {
            problems.push(format!("expected a diagnostic `{marker}`; there is none"));
        }
    }
    for text in &expectation.texts {
        if !stderr.contains(text) {
            problems.push(format!("expected the output to contain {text:?}"));
        }
    }
    for text in &expectation.absent {
        if stderr.contains(text) {
            problems.push(format!("expected the output NOT to contain {text:?}"));
        }
    }

    if !problems.is_empty() {
        let listing: Vec<String> = problems
            .iter()
            .map(|problem| format!("  - {problem}"))
            .collect();
        panic!(
            "case `{name}` ({}) was rejected, but not as expected:\n{}\n\n--- stderr ---\n{stderr}",
            program.display(),
            listing.join("\n")
        );
    }
}

// ===== panic luc chay =====

/// What a `.panic` file asks of a program that is expected to die.
#[derive(Debug, Default)]
pub struct PanicExpectation {
    pub status: Option<i32>,
    pub stdout: Vec<String>,
    pub stderr: Vec<String>,
}

/// Runs `tests/cases/panic/NAME.pump`, asserting it dies the way
/// `tests/cases/panic/NAME.panic` describes.
pub fn check_panic_case(name: &str) {
    let directory = cases_dir("panic");
    let program = directory.join(format!("{name}.pump"));
    let expectation = parse_panic_expectation(&directory.join(format!("{name}.panic")));

    let output = invoke(&["run", &program.display().to_string()]);
    let stdout = normalise(&String::from_utf8_lossy(&output.stdout));
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    let mut problems = Vec::new();
    if let Some(status) = expectation.status {
        if output.status.code() != Some(status) {
            problems.push(format!(
                "expected exit status {status}, and it was {}",
                output.status
            ));
        }
    }
    let expected_stdout = normalise(&expectation.stdout.join(
        "
",
    ));
    if stdout != expected_stdout {
        problems.push(format!(
            "expected stdout {expected_stdout:?}, and it was {stdout:?}"
        ));
    }
    for text in &expectation.stderr {
        if !stderr.contains(text) {
            problems.push(format!("expected the panic message to contain {text:?}"));
        }
    }

    if !problems.is_empty() {
        let listing: Vec<String> = problems
            .iter()
            .map(|problem| format!("  - {problem}"))
            .collect();
        panic!(
            "case `{name}` ({}) did not fail as expected:
{}

--- stderr ---
{stderr}",
            program.display(),
            listing.join(
                "
"
            )
        );
    }
}

fn parse_panic_expectation(path: &Path) -> PanicExpectation {
    let source = read(path);
    let mut expectation = PanicExpectation::default();

    for (index, line) in source.lines().enumerate() {
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() || trimmed.trim_start().starts_with('#') {
            continue;
        }
        let (key, value) = trimmed
            .trim_start()
            .split_once(char::is_whitespace)
            .unwrap_or((trimmed.trim_start(), ""));
        match key {
            "status" => {
                expectation.status = Some(value.trim().parse().unwrap_or_else(|error| {
                    panic!(
                        "{}:{}: `status` needs a number: {error}",
                        path.display(),
                        index + 1
                    )
                }));
            }
            "out" => expectation.stdout.push(value.trim_start().to_string()),
            "err" => expectation.stderr.push(value.trim().to_string()),
            other => panic!(
                "{}:{}: unknown key `{other}`; the keys are `status`, `out` and `err`",
                path.display(),
                index + 1
            ),
        }
    }

    assert!(
        !expectation.stderr.is_empty(),
        "{}: a panic expectation must name the message with at least one `err`",
        path.display()
    );
    expectation
}

// ===== so dang ky =====
