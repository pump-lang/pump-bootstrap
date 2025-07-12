// `pump run`. Dich thang vao bo nho roi nhay vao chay luon.
//
// cai nay khong co file
// pump-runtime, pumpc link no nhu mot dependency binh thuong.
//
// Nho: phai dang ky HET symbol vao builder truoc khi define bat cu thu gi,
// khong thi linker no vut mat may entry point ma chi code sinh ra moi goi.
// Cho nay t mat nguyen mot toi.

use std::ffi::CString;

use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{FuncOrDataId, Module};

use crate::abi;
use crate::clif::{self, CodegenOptions};
use crate::errors::{CompileError, ErrorCode};
use crate::ir::Program;
use crate::token::Span;
use crate::Options;

// `%` cua Rust tren f64 chinh la fmod cua C, may man the.
extern "C" fn fmod_shim(a: f64, b: f64) -> f64 {
    a % b
}

/// Compiles the program into memory and runs it. Gives back the exit code.
pub fn run(program: &Program, options: &Options) -> Result<u8, CompileError> {
    let codegen = CodegenOptions {
        dump_clif: options.dump_clif,
        // ban JIT tu lam phan khoi dong, khong di qua `main` cua C
        emit_c_main: false,
        ..CodegenOptions::default()
    };

    let mut m = build_module(&codegen)?;
    clif::define_program(&mut m, program, &codegen)?;
    m.finalize_definitions()
        .map_err(|e| failure(format!("cannot finalise the compiled program: {e}")))?;

    let type_table = data_address(&m, abi::SYMBOL_TYPE_TABLE)?;
    let global_roots = data_address(&m, abi::SYMBOL_GLOBAL_ROOTS)? as *mut *mut u8;
    let init = function_address(&m, abi::SYMBOL_MODULE_INIT)?;
    let pmain = function_address(&m, abi::SYMBOL_PROGRAM_MAIN)?;
    // println!("DBG: init={:?} main={:?}", init, pmain);

    // SAFETY: hai symbol nay vua duoc sinh ra voi dung chu ky backend co dinh,
    // `void()` va `int32_t()`. docs/abi.md muc 7.
    let init: extern "C" fn() = unsafe { std::mem::transmute(init) };
    let pmain: extern "C" fn() -> i32 = unsafe { std::mem::transmute(pmain) };

    // `args` giu that su may byte ma vector con tro chi vao, nen no phai song
    // lau hon ca lan chay.
    let args = command_line(options);
    let mut argv: Vec<*const u8> = args.iter().map(|a| a.as_ptr() as *const u8).collect();
    argv.push(std::ptr::null());
    let argc = args.len() as i32;

    // dia chi cua bien nay la dau xa cua vung stack ma GC quet kieu bao thu:
    // moi frame cua Pump deu nam duoi no.
    let mut anchor: u64 = 0;
    let stack_bottom = std::ptr::addr_of_mut!(anchor) as *const u8;

    pump_runtime::start::pump_rt_init(
        stack_bottom,
        type_table as *const pump_runtime::TypeDescriptor,
        program.type_descriptors.len() as u64,
        global_roots,
        program.root_globals().count() as u64,
        argc,
        argv.as_ptr(),
    );
    init();
    let code = pmain();
    pump_runtime::start::pump_rt_shutdown(code);

    // code da chet roi nhung module van giu may trang nho no nam trong, nen
    // de no song den tan day.
    drop(m);
    Ok(code as u8)
}

fn build_module(options: &CodegenOptions) -> Result<JITModule, CompileError> {
    let mut b = JITBuilder::with_flags(
        &[("opt_level", options.opt_level)],
        cranelift_module::default_libcall_names(),
    )
    .map_err(|e| failure(format!("cannot start the JIT: {e}")))?;

    for (name, address) in pump_runtime::runtime_symbols() {
        b.symbol(name, address);
    }
    b.symbol(clif::SYMBOL_FMOD, fmod_shim as *const () as *const u8);

    Ok(JITModule::new(b))
}

// argv[0]: lay file entry, `pump run` khong co ten chuong trinh nao khac.
// Tu argv[1] la nhung gi nguoi ta viet sau `--`, chuong trinh nhan lai bang
// `os.args()`. Doi so nao co NUL o giua thi thanh chuoi rong, vi argv cua C
// ket thuc bang NUL nen khong mang duoc no.
fn command_line(options: &Options) -> Vec<CString> {
    let entry = options.entry.to_string_lossy().into_owned();
    let mut line = vec![CString::new(entry).unwrap_or_default()];
    for argument in &options.program_args {
        line.push(CString::new(argument.as_str()).unwrap_or_default());
    }
    line
}

fn data_address(m: &JITModule, name: &str) -> Result<*const u8, CompileError> {
    match m.get_name(name) {
        Some(FuncOrDataId::Data(id)) => Ok(m.get_finalized_data(id).0),
        _ => Err(failure(format!(
            "the compiled program has no `{name}` data"
        ))),
    }
}

fn function_address(m: &JITModule, name: &str) -> Result<*const u8, CompileError> {
    match m.get_name(name) {
        Some(FuncOrDataId::Func(id)) => Ok(m.get_finalized_function(id)),
        _ => Err(failure(format!("the compiled program has no `{name}`"))),
    }
}

fn failure(message: impl Into<String>) -> CompileError {
    CompileError::at(ErrorCode::CodegenFailed, Span::synthetic(), message)
}
