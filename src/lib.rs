// compiler cua Pump.
//
//   .pump -> lexer -> parser -> resolve -> check -> lower -> IR
//                                                            |
//                                      +---------------------+
//                                      |
//                                   clif.rs
//                                    /    \
//                                jit.rs   object + link.rs -> .exe
//
// ai doc cay nay lan dau thi doc theo thu tu nay: grammar/pump.ebnf, roi
// token voi ast, roi types, roi abi doc kem docs/abi.md, cuoi cung la ir.

pub mod abi;
pub mod ast;
pub mod check;
pub mod clif;
pub mod errors;
pub mod ir;
pub mod jit;
pub mod lexer;
pub mod link;
pub mod lower;
pub mod parser;
pub mod resolve;
pub mod token;
pub mod types;

use std::path::{Path, PathBuf};

use crate::ast::NodeIdAllocator;
use crate::errors::{CompileError, Diagnostics, ErrorCode, SourceMap};
use crate::ir::Program;
use crate::token::Span;

/// What the driver was asked to make.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Run,
    Build,
}

/// One request to compile something.
#[derive(Clone, Debug)]
pub struct Options {
    pub mode: Mode,
    pub entry: PathBuf,
    pub output: Option<PathBuf>,
    pub dump_ir: bool,
    pub dump_clif: bool,
    /// What `pump run FILE -- ...` hands to the program itself, which reads
    /// it back through `os.args()`. Never options of the compiler.
    pub program_args: Vec<String>,
}

impl Options {
    pub fn new(mode: Mode, entry: impl Into<PathBuf>) -> Options {
        Options {
            mode,
            entry: entry.into(),
            output: None,
            dump_ir: false,
            dump_clif: false,
            program_args: Vec::new(),
        }
    }

    /// Goc cua project, tuc la thu muc chua file entry.
    pub fn project_root(&self) -> &Path {
        self.entry.parent().unwrap_or_else(|| Path::new("."))
    }

    /// Cho ma `build` ghi file chay ra.
    pub fn executable_path(&self) -> PathBuf {
        if let Some(output) = &self.output {
            return output.clone();
        }
        let stem = self
            .entry
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "a".to_string());
        self.project_root()
            .join(stem)
            .with_extension(std::env::consts::EXE_EXTENSION)
    }
}

/// Everything one compile piles up: source text, diagnostics, node ids.
#[derive(Debug, Default)]
pub struct Session {
    pub sources: SourceMap,
    pub diagnostics: Diagnostics,
    pub node_ids: NodeIdAllocator,
}

impl Session {
    pub fn new() -> Session {
        Session::default()
    }

    /// Read a file into the source map. Hong thi bao E0700.
    pub fn load(&mut self, path: &Path) -> Result<token::FileId, CompileError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(self.sources.add(path, text)),
            Err(error) => Err(CompileError::at(
                ErrorCode::CannotReadFile,
                Span::synthetic(),
                format!("cannot read `{}`: {error}", path.display()),
            )),
        }
    }

    /// Draw out every diagnostic so far, xep theo vi tri.
    pub fn render_diagnostics(&mut self) -> String {
        self.diagnostics.sort();
        self.diagnostics.render(&self.sources)
    }
}

/// Run the frontend and the lowering, gives back the IR of options.entry.
pub fn compile_to_ir(session: &mut Session, options: &Options) -> Result<Program, CompileError> {
    let entry_file = session.load(&options.entry)?;
    let entry_text = session
        .sources
        .get(entry_file)
        .expect("the entry file was just loaded")
        .text
        .clone();

    let module_name = options
        .entry
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "main".to_string());

    let tokens = lexer::tokenize(entry_file, &entry_text, &mut session.diagnostics);
    let unit = parser::parse(
        entry_file,
        vec![module_name],
        &tokens,
        &mut session.node_ids,
        &mut session.diagnostics,
    );

    // file parse hong thi day cho trong do recovery de lai. Cho may pha sau
    // chay tiep tren no chi ra them mot dong loi an theo chu khong biet them
    // gi, nen cu het mot pha la dung neu da co loi.
    if session.diagnostics.has_errors() {
        return Err(stopped_early());
    }

    // module import thi resolver tu doc, tu lex, tu parse, vi no la pha duy
    // nhat biet do thi import. No can &mut Session de ghi source map nen loi
    // cua no roi vao mot cho khac, xong xuoi moi don nguoc lai vao day.
    let mut resolver_diagnostics = Diagnostics::new();
    let resolution = resolve::resolve(
        vec![unit],
        options.project_root(),
        session,
        &mut resolver_diagnostics,
    );
    session.diagnostics.extend(resolver_diagnostics);
    let resolution = resolution?;
    if session.diagnostics.has_errors() {
        return Err(stopped_early());
    }

    let checked = check::check(resolution, &mut session.diagnostics)?;
    if session.diagnostics.has_errors() {
        return Err(stopped_early());
    }

    lower::lower(&checked)
}

fn stopped_early() -> CompileError {
    CompileError::at(
        ErrorCode::CompilationStopped,
        Span::synthetic(),
        String::new(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_output_sits_beside_the_entry_file() {
        let options = Options::new(Mode::Build, "examples/hello.pump");
        let executable = options.executable_path();
        assert_eq!(executable.parent().unwrap(), Path::new("examples"));
        assert!(executable.file_stem().is_some_and(|stem| stem == "hello"));
    }
}
