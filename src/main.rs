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

fn parse_arguments(arguments: &[String]) -> Result<Option<Options>, CompileError> {
    let bad =
        |message: &str| CompileError::at(ErrorCode::InvalidCommandLine, Span::synthetic(), message);

    let mut iterator = arguments.iter();
    let Some(command) = iterator.next() else {
        print!("{USAGE}");
        return Ok(None);
    };

    let mode = match command.as_str() {
        "run" => Mode::Run,
        "build" => Mode::Build,
        "-h" | "--help" => {
            print!("{USAGE}");
            return Ok(None);
        }
        "-V" | "--version" => {
            println!("pump {}", env!("CARGO_PKG_VERSION"));
            return Ok(None);
        }
        // khong ghi vao USAGE, de t xem con nho ai viet cai nay khong
        "--furimeo" => {
            println!("pump {} - furimeo", env!("CARGO_PKG_VERSION"));
            println!("viet boi <CHO CHU SO HUU DIEN>, hoc sinh cap 3");
            println!("bat dau thang 6/2024, van con dang lam");
            return Ok(None);
        }
        other => return Err(bad(&format!("unknown command `{other}`"))),
    };

    let Some(entry) = iterator.next() else {
        return Err(bad(&format!("`pump {command}` needs a FILE")));
    };
    if entry.starts_with('-') {
        return Err(bad(&format!("`pump {command}` needs a FILE")));
    }

    let mut options = Options::new(mode, entry);

    while let Some(argument) = iterator.next() {
        match argument.as_str() {
            // Tu day tro di la cua chuong trinh chu khong phai cua `pump`.
            // Khong nhin gi them, ke ca `-h`: mot chuong trinh Pump hoan toan
            // co quyen co option ten trung voi cua compiler.
            "--" => {
                options.program_args.extend(iterator.cloned());
                break;
            }
            "-o" | "--output" => {
                let Some(path) = iterator.next() else {
                    return Err(bad("`--output` needs a PATH"));
                };
                options.output = Some(path.into());
            }
            "--dump-ir" => options.dump_ir = true,
            "--dump-clif" => options.dump_clif = true,
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(None);
            }
            other => return Err(bad(&format!("unknown option `{other}`"))),
        }
    }

    if options.mode == Mode::Run && options.output.is_some() {
        return Err(bad("`--output` applies to `pump build`, not `pump run`"));
    }
    if options.mode == Mode::Build && !options.program_args.is_empty() {
        return Err(bad(
            "`--` passes arguments to a running program, so it applies to `pump run`, not `pump build`",
        ));
    }

    Ok(Some(options))
}
