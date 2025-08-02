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

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();

    let options = match parse_arguments(&arguments) {
        Ok(Some(options)) => options,
        // hoi help hoac version thi in xong roi, thoat em dep
        Ok(None) => return ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error[{}]: {}", error.code, error.message);
            eprint!("\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };

    let mut session = Session::new();
    let result = pumpc::compile_to_ir(&mut session, &options);

    let program = match result {
        Ok(program) => program,
        Err(error) => {
            // pha nao dung lai vi pha truoc da bao loi thi tra ve mot loi
            // rong, khong co gi de them. Loi that nam trong session roi.
            if !session.diagnostics.has_errors() {
                session.diagnostics.push(error);
            }
            eprint!("{}", session.render_diagnostics());
            return ExitCode::FAILURE;
        }
    };

    if session.diagnostics.has_errors() {
        eprint!("{}", session.render_diagnostics());
        return ExitCode::FAILURE;
    }
    if !session.diagnostics.is_empty() {
        eprint!("{}", session.render_diagnostics());
    }

    if options.dump_ir {
        eprintln!("{program:#?}");
    }

    let outcome = match options.mode {
        Mode::Run => pumpc::jit::run(&program, &options).map(ExitCode::from),
        Mode::Build => pumpc::link::build_executable(&program, &options).map(|_| ExitCode::SUCCESS),
    };

    match outcome {
        Ok(code) => code,
        Err(error) => {
            session.diagnostics.push(error);
            eprint!("{}", session.render_diagnostics());
            ExitCode::FAILURE
        }
    }
}
