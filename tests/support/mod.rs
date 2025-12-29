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

/// Asserts that the case files on disk are exactly the ones the suite lists,
/// so that neither a stray program nor a missing expectation goes unnoticed.
pub fn check_registry(kind: &str, answer_extension: &str, listed: &[&str]) {
    let directory = cases_dir(kind);
    let mut programs = Vec::new();
    let mut answers = Vec::new();

    let entries = std::fs::read_dir(&directory)
        .unwrap_or_else(|error| panic!("cannot read `{}`: {error}", directory.display()));
    for entry in entries {
        let path = entry.expect("a readable directory entry").path();
        let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
            continue;
        };
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .expect("a case file name is valid UTF-8")
            .to_string();
        if extension == "pump" {
            programs.push(stem);
        } else if extension == answer_extension {
            answers.push(stem);
        }
    }

    programs.sort();
    answers.sort();
    let mut listed: Vec<String> = listed.iter().map(|name| (*name).to_string()).collect();
    listed.sort();

    assert_eq!(
        programs, listed,
        "the `{kind}` programs on disk are not the cases the suite lists"
    );
    assert_eq!(
        answers, listed,
        "every `{kind}` program needs exactly one `.{answer_extension}` beside it"
    );
}

/// The `project` variant: one directory per case, each holding `main.pump`.
pub fn check_project_registry(listed: &[&str]) {
    let directory = cases_dir("project");
    let mut found = Vec::new();

    let entries = std::fs::read_dir(&directory)
        .unwrap_or_else(|error| panic!("cannot read `{}`: {error}", directory.display()));
    for entry in entries {
        let path = entry.expect("a readable directory entry").path();
        if !path.is_dir() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .expect("a case directory name is valid UTF-8")
            .to_string();
        assert!(
            path.join("main.pump").is_file(),
            "project case `{name}` has no `main.pump`"
        );
        assert!(
            path.join("expected.out").is_file(),
            "project case `{name}` has no `expected.out`"
        );
        found.push(name);
    }

    found.sort();
    let mut listed: Vec<String> = listed.iter().map(|name| (*name).to_string()).collect();
    listed.sort();
    assert_eq!(
        found, listed,
        "the project case directories are not the cases the suite lists"
    );
}

// ===== in ket qua ra =====

fn read(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("cannot read `{}`: {error}", path.display()))
}

fn normalise(text: &str) -> String {
    let mut lines: Vec<&str> = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .map(str::trim_end)
        .collect();
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

fn failure_report(name: &str, program: &Path, headline: &str, output: &Output) -> String {
    format!(
        "case `{name}` ({}): {headline}.\nexit status: {}\n--- stdout ---\n{}--- stderr ---\n{}",
        program.display(),
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    )
}

fn diff_report(
    name: &str,
    program: &Path,
    expected: &str,
    actual: &str,
    output: &Output,
) -> String {
    let expected_lines: Vec<&str> = expected.lines().collect();
    let actual_lines: Vec<&str> = actual.lines().collect();

    let mut rows = Vec::new();
    for index in 0..expected_lines.len().max(actual_lines.len()) {
        let want = expected_lines.get(index).copied();
        let got = actual_lines.get(index).copied();
        let marker = if want == got { ' ' } else { '!' };
        rows.push(format!(
            "{marker} {:>3} | expected {:<44} | actual {}",
            index + 1,
            show(want),
            show(got)
        ));
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let trailer = if stderr.trim().is_empty() {
        String::new()
    } else {
        format!("\n--- stderr ---\n{stderr}")
    };

    format!(
        "case `{name}` ({}) produced the wrong output:\n{}{trailer}",
        program.display(),
        rows.join("\n")
    )
}

fn show(line: Option<&str>) -> String {
    match line {
        Some(line) => format!("{line:?}"),
        None => "<none>".to_string(),
    }
}
