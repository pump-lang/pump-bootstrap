// `pump build`: mot file object cong rust-lld ra mot cai .exe.
//
// rust-lld di kem rustup san roi nen khong phai cai them gi:
//
//   ~/.rustup/toolchains/<toolchain>/lib/rustlib/<host>/bin/rust-lld.exe
//
// Tren target nay no phai chay o che do link.exe, va entry point la
// `mainCRTStartup`, cai nay se goi `main` do compiler sinh ra. Chinh vi vay
// ma runtime co y khong dinh nghia mot cai `main` nao cua rieng no.
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
    let search_paths = library_search_paths()?;

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

fn library_search_paths() -> Result<Vec<PathBuf>, CompileError> {
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

fn msvc_library_directory() -> Option<PathBuf> {
    let install = visual_studio_install()?;
    let version_file = install
        .join("VC")
        .join("Auxiliary")
        .join("Build")
        .join("Microsoft.VCToolsVersion.default.txt");
    let version = std::fs::read_to_string(version_file).ok()?;
    let directory = install
        .join("VC")
        .join("Tools")
        .join("MSVC")
        .join(version.trim())
        .join("lib")
        .join("x64");
    directory.is_dir().then_some(directory)
}

fn visual_studio_install() -> Option<PathBuf> {
    let program_files =
        std::env::var_os("ProgramFiles(x86)").unwrap_or_else(|| r"C:\Program Files (x86)".into());
    let vswhere = PathBuf::from(program_files)
        .join("Microsoft Visual Studio")
        .join("Installer")
        .join("vswhere.exe");
    if !vswhere.is_file() {
        return None;
    }
    let output = Command::new(vswhere)
        .args([
            "-latest",
            "-products",
            "*",
            "-requires",
            "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
            "-property",
            "installationPath",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8(output.stdout).ok()?;
    let path = path.trim();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

fn windows_sdk_library_directories() -> Vec<PathBuf> {
    let Some(root) = windows_sdk_root() else {
        return Vec::new();
    };
    let libraries = root.join("Lib");
    let Ok(entries) = std::fs::read_dir(&libraries) else {
        return Vec::new();
    };

    let mut versions: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.join("ucrt").join("x64").is_dir())
        .collect();
    versions.sort();

    match versions.pop() {
        Some(newest) => vec![
            newest.join("ucrt").join("x64"),
            newest.join("um").join("x64"),
        ],
        None => Vec::new(),
    }
}

fn windows_sdk_root() -> Option<PathBuf> {
    let output = Command::new("reg")
        .args([
            r"query",
            r"HKLM\SOFTWARE\Microsoft\Windows Kits\Installed Roots",
            "/v",
            "KitsRoot10",
            "/reg:32",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let value = text
        .lines()
        .find_map(|line| line.split("REG_SZ").nth(1))?
        .trim();
    (!value.is_empty()).then(|| PathBuf::from(value))
}

fn find_msvc_link() -> Option<PathBuf> {
    let directory = msvc_library_directory()?;
    // .../lib/x64 -> .../bin/Hostx64/x64/link.exe
    let toolset = directory.parent()?.parent()?;
    let link = toolset
        .join("bin")
        .join("Hostx64")
        .join("x64")
        .join("link.exe");
    link.is_file().then_some(link)
}

// bao loi ra ngoai

fn linker_diagnostics(stdout: &[u8], stderr: &[u8]) -> Vec<String> {
    const LIMIT: usize = 10;

    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(stderr),
        String::from_utf8_lossy(stdout)
    );

    let mut interesting: Vec<String> = Vec::new();
    for line in combined.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let lowered = line.to_ascii_lowercase();
        let is_diagnostic = lowered.contains("error")
            || lowered.contains("warning")
            || lowered.contains("undefined symbol")
            || lowered.contains("cannot open");
        if is_diagnostic && !interesting.iter().any(|kept| kept == line) {
            interesting.push(line.to_string());
        }
        if interesting.len() == LIMIT {
            interesting.push("... and more".to_string());
            break;
        }
    }

    if interesting.is_empty() {
        // khong dong nao khop bo loc, tuc la linker noi mot cai gi la, thi
        // dua nguyen ca cuc ra van la co ich nhat.
        let trimmed = combined.trim();
        if trimmed.is_empty() {
            return vec!["the linker failed without saying why".to_string()];
        }
        return trimmed.lines().take(LIMIT).map(str::to_string).collect();
    }
    interesting
}

fn missing_runtime(detail: &str) -> CompileError {
    CompileError::at( ErrorCode::RuntimeLibraryNotFound, Span::synthetic(), "cannot find `pump_runtime.lib`", )
    .with_note(detail)
    .with_note("a plain `cargo build` builds the runtime only as an rlib")
    .with_help(
        "run `cargo build --release --workspace`, or point `PUMP_RUNTIME_LIB` at the library",
    )
}

fn write_failure(path: &Path, detail: &str) -> CompileError {
    CompileError::at(
        ErrorCode::CannotWriteFile,
        Span::synthetic(),
        format!("cannot write `{}`: {detail}", path.display()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_diagnostic_lines_survive() {
        let stderr = b"rust-lld: error: undefined symbol: pump_alloc\n\nsome noise\n";
        let notes = linker_diagnostics(stderr, b"");
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("pump_alloc"));
    }

    #[test]
    fn a_repeated_error_is_reported_once() {
        let stderr = b"lld: error: duplicate symbol: main\nlld: error: duplicate symbol: main\n";
        assert_eq!(linker_diagnostics(stderr, b"").len(), 1);
    }

    #[test]
    fn silence_still_produces_a_note() {
        assert_eq!(linker_diagnostics(b"", b"").len(), 1);
    }
}
