// GC, thu tu dau den cuoi.
//
// Moi ca trong tests/gc la mot chuong trinh Pump cap phat nhieu gap may lan
// suc chua cua heap, nen GC chay di chay lai trong luc no thuc thi, roi in ra
// checksum cua nhung du lieu le ra phai song. File ket qua nam canh la cai ma
// mot bo GC dung se in ra; bo GC nao gom nham mot thu con song thi mot con so
// se lech chu khong nhat thiet la do, va do la ca cai y cua hinh nay.
//
// May ca nay CHAM co y: chung sinh ra de dan GC, ma dan thi phai co thoi
// gian. Day la nua ton kem nhat trong ca cay test.

mod support;

use std::path::{Path, PathBuf};

use support::{invoke, project_root};

const CASES: &[&str] = &[
    "allocation_pressure",
    "closure_captures",
    "collections_under_pressure",
    "deep_recursion",
    "liveness",
    "object_graph",
    "reference_cycles",
    "string_churn",
];

fn gc_dir() -> PathBuf {
    project_root().join("tests").join("gc")
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

fn read(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("cannot read `{}`: {error}", path.display()))
}

fn run_case(name: &str) -> String {
    let program = gc_dir().join(format!("{name}.pump"));
    let output = invoke(&["run", &program.display().to_string()]);
    assert!(
        output.status.success(),
        "gc case `{name}` ({}) did not complete: exit status {}\n--- stdout ---\n{}\
         --- stderr ---\n{}",
        program.display(),
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    normalise(&String::from_utf8_lossy(&output.stdout))
}

fn check_case(name: &str) {
    let actual = run_case(name);
    let expected = normalise(&read(&gc_dir().join(format!("{name}.out"))));
    if actual == expected {
        return;
    }

    let expected_lines: Vec<&str> = expected.lines().collect();
    let actual_lines: Vec<&str> = actual.lines().collect();
    let mut rows = Vec::new();
    for index in 0..expected_lines.len().max(actual_lines.len()) {
        let want = expected_lines.get(index).copied().unwrap_or("<none>");
        let got = actual_lines.get(index).copied().unwrap_or("<none>");
        let marker = if want == got { ' ' } else { '!' };
        rows.push(format!(
            "{marker} {:>3} | expected {want:<24} | actual {got}",
            index + 1
        ));
    }
    panic!(
        "gc case `{name}` produced the wrong answer, which means the collector \
         freed or corrupted something live:\n{}",
        rows.join("\n")
    );
}

#[test]
fn allocation_pressure() {
    check_case("allocation_pressure");
}

#[test]
fn reference_cycles() {
    check_case("reference_cycles");
}

#[test]
fn liveness() {
    check_case("liveness");
}

#[test]
fn deep_recursion() {
    check_case("deep_recursion");
}

#[test]
fn collections_under_pressure() {
    check_case("collections_under_pressure");
}

#[test]
fn string_churn() {
    check_case("string_churn");
}

#[test]
fn closure_captures() {
    check_case("closure_captures");
}

#[test]
fn object_graph() {
    check_case("object_graph");
}

#[test]
fn repeated_runs_agree() {
    let first = run_case("reference_cycles");
    for attempt in 2..=3 {
        let again = run_case("reference_cycles");
        assert_eq!(
            first, again,
            "run {attempt} of `reference_cycles` disagreed with run 1"
        );
    }
}

#[test]
fn every_case_is_registered() {
    let directory = gc_dir();
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
        match extension {
            "pump" => programs.push(stem),
            "out" => answers.push(stem),
            _ => {}
        }
    }

    programs.sort();
    answers.sort();
    let mut listed: Vec<String> = CASES.iter().map(|name| (*name).to_string()).collect();
    listed.sort();

    assert_eq!(
        programs, listed,
        "the programs in `tests/gc` are not the cases this suite lists"
    );
    assert_eq!(
        answers, listed,
        "every program in `tests/gc` needs exactly one `.out` beside it"
    );
}
