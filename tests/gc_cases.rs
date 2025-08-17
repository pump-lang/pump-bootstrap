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
