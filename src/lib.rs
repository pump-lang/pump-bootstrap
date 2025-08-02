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
