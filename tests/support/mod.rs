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
// stderr cua no, ...

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

// ===== ca phai
