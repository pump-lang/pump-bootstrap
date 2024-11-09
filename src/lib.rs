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
