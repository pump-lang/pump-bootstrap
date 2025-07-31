// cai nay `pump build`: mot
//
// rust-lld di kem rustup san roi nen khong phai cai them gi:
//
//   ~/.rustup/toolchains/<toolchain>/lib/rustlib/<host>/bin/rust-lld.exe
//
// Tren target nay no phai chay o
// `mainCRTStartup`, cai nay se goi `main` do compiler sinh ra. Chinh vi vay
// ma runtime co ...
//
// Nua xau xi cua file la doan di tim CRT. O trong developer command prompt
// thi bien LIB co san, cu the ma dung. O ngoai thi vswhere tim cho cai Visual
// Studio, registry tim cho cai Windows SDK, hai duong deu ve dung ba tham so
// /LIBPATH ma developer prompt dua cho minh.
//
// Link hong thi nem nguyen stderr cua linker vao thong bao loi. Chi dua moi
// ma thoat ra thi nguoi phai sua no biet duong nao ma lan.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::clif::{self, CodegenOptions};
use crate::errors::{CompileError, ErrorCode};
use crate::ir::Program;
use crate::token::Span;
use crate::Options;

const SYSTEM_LIBRARIES: &[&str] = &[
    "kernel32.lib",
    "advapi32.lib",
    "userenv.lib",
    "ws2_32.lib",
    "bcrypt.lib",
    "ntdll.lib",
    "synchronization.lib",
    "msvcrt.lib",
    "ucrt.lib",
    "vcruntime.lib",
    "legacy_stdio_definitions.lib",
];

/// Find rust-lld inside the rustup toolchain in use.
pub fn find_linker() -> Result<PathBuf, CompileError> {
    if let Some(sysroot) = rustc_sysroot() {
        let rustlib = sysroot.join("lib").join("rustlib");
        let host = target_lexicon::Triple::host().to_string();
        let preferred = rustlib.join(&host).join("bin").join("rust-lld.exe");
        if preferred.is_file() {
            return Ok(preferred);
        }
        // thu muc toolchain bi doi ten hoac la cross: lay bua thu muc
        // target nao co that cai linker o trong.
        if let Ok(entries) = std::fs::read_dir(&rustlib) {
            for entry in entries.flatten() {
                let candidate = entry.path().join("bin").join("rust-lld.exe");
                if candidate.is_file() {
                    return Ok(candidate);
                }
            }
        }
    }

    if let Some(link_exe) = find_msvc_link() {
        return Ok(link_exe);
    }

    Err(CompileError::at(
        ErrorCode::LinkerNotFound,
        Span::synthetic(),
        "cannot find a linker",
    )
    .with_note(
        "`rust-lld.exe` normally lives in \
         <sysroot>/lib/rustlib/<target>/bin, where <sysroot> is what \
         `rustc --print sysroot` prints",
    )
    .with_help(
        "run `rustup component add llvm-tools` or install the Visual Studio C++ build tools",
    ))
}

/// Find the pump-runtime static library we link against.
pub fn find_runtime_library() -> Result<PathBuf, CompileError> {
    if let Some(configured) = std::env::var_os("PUMP_RUNTIME_LIB") {
        let path = PathBuf::from(configured);
        if path.is_file() {
            return Ok(path);
        }
        return Err(missing_runtime(&format!(
            "`PUMP_RUNTIME_LIB` points at `{}`, which is not a file",
            path.display()
        )));
    }

    let mut searched = Vec::new();
    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            searched.push(directory.join("pump_runtime.lib"));
            searched.push(directory.join("deps").join("pump_runtime.lib"));
        }
    }
    for profile in ["debug", "release"] {
        searched.push(Path::new("target").join(profile).join("pump_runtime.lib"));
    }

    if let Some(found) = searched.iter().find(|path| path.is_file()) {
        return Ok(found.clone());
    }

    let listing: Vec<String> = searched
        .iter()
        .map(|path| format!("  {}", path.display()))
        .collect();
    Err(missing_runtime(&format!(
        "looked in:\n{}",
        listing.join("\n")
    )))
}

/// Link one object file into an executable.
pub fn link_executable(
    object: &Path,
    runtime_library: &Path,
    output: &Path,
) -> Result<(), CompileError> {
    let linker = find_linker()?;
    let search_paths = library2()?;

    let mut command = Command::new(&linker);
    if linker
        .file_stem()
        .is_some_and(|stem| stem.eq_ignore_ascii_case(OsStr::new("rust-lld")))
    {
        command.args(["-flavor", "link"]);
    }
    command.arg("/NOLOGO");
    command.arg("/MACHINE:X64");
    command.arg("/SUBSYSTEM:CONSOLE");
    command.arg("/ENTRY:mainCRTStartup");
    command.arg(format!("/OUT:{}", output.display()));
    for path in &search_paths {
        command.arg(format!("/LIBPATH:{}", path.display()));
    }
    command.arg(object);
    command.arg(runtime_library);
    command.args(SYSTEM_LIBRARIES);

    let outcome = command.output().map_err(|error| {
        CompileError::at(
            ErrorCode::LinkerNotFound,
            Span::synthetic(),
            format!("cannot run `{}`: {error}", linker.display()),
        )
    })?;

    if outcome.status.success() {
        return Ok(());
    }

    let mut error = CompileError::at(
        ErrorCode::LinkFailed,
        Span::synthetic(),
        format!(
            "`{}` could not link `{}`",
            linker.display(),
            output.display()
        ),
    );
    for note in linker_diagnostics(&outcome.stdout, &outcome.stderr) {
        error = error.with_note(note);
    }
    if search_paths.is_empty() {
        error = error.with_help(
            "no C runtime library directories were found; run `pump build` from a \
             Visual Studio developer command prompt, or set `LIB`",
        );
    }
    Err(error)
}

/// Compile and link the program, write the executable where the options
/// say.
pub fn build_executable(program: &Program, options: &Options) -> Result<PathBuf, CompileError> {
    let codegen = CodegenOptions {
        dump_clif: options.dump_clif,
        emit_c_main: true,
        ..CodegenOptions::default()
    };
    let object = clif::emit_object(program, &codegen)?;

    let output = options.executable_path();
    if let Some(directory) = output.parent() {
        if !directory.as_os_str().is_empty() {
            std::fs::create_dir_all(directory).map_err(|error| {
                write_failure(directory, &format!("cannot create the directory: {error}"))
            })?;
        }
    }

    let object_path = output.with_extension("obj");
    std::fs::write(&object_path, &object)
        .map_err(|error| write_failure(&object_path, &error.to_string()))?;

    let runtime_library = find_runtime_library()?;
    let result = link_executable(&object_path, &runtime_library, &output);
    // file object chi la gian giao, de lai thi ban goc project. Link hong
    // thi giu, vi luc do no chinh la thu can mo ra xem.
    if result.is_ok() {
        let _ = std::fs::remove_file(&object_path);
    }
    result.map(|()| output)
}

// di tim toolchain

fn rustc_sysroot() -> Option<PathBuf> {
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let output = Command::new(rustc)
        .arg("--print")
        .arg("sysroot")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8(output.stdout).ok()?;
    let path = path.trim();
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}

fn library2() -> Result<Vec<PathBuf>, CompileError> {
    if let Some(configured) = std::env::var_os("LIB") {
        let paths: Vec<PathBuf> = std::env::split_paths(&configured)
            .filter(|path| !path.as_os_str().is_empty())
            .collect();
        if !paths.is_empty() {
            return Ok(paths);
        }
    }

    let mut paths = Vec::new();
    if let Some(directory) = msvc_library_directory() {
        paths.push(directory);
    }
    paths.extend(windows_sdk_library_directories());
    Ok(paths)
}
