// dong lenh cua `pump`.
//
//   pump run   FILE [-- ARGS] dich trong bo nho roi chay
//   pump build FILE [-o OUT]   dich va link ra .exe

use std::process::ExitCode;

use pumpc::errors::{CompileError, ErrorCode};
use pumpc::token::Span;
use pumpc::{Mode, Options, Session};

const USAGE: &str = "\
pump - the Pump compiler

usage:
    pump run   FILE [options] [-- ARGS]   compile in memory and run immediately
    pump build FILE [options]             compile and link an executable

options:
    -o, --output PATH             where `build` writes the executable
        --dump-ir                 print the mid-level IR to stderr
        --dump-clif               print the generated Cranelift IR to stderr
    -h, --help                    show this message
    -V, --version                 show the version

everything after `--` goes to the program being run, not to `pump`, and it
reads it back with `os.args()`.
";
