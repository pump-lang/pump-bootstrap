// sinh ma bang Cranelift. mot ir::Program thanh ma may.
//
// CANH BAO CHO CHINH T SAU NAY: file nay ban nhat repo. T viet no trong hai
// tuan he, vua xem video vua mo doc, va co nhung doan t chi biet la no chay
// chu khong biet han tai sao. Doan nao t chua chac thi t de lai ban cu ngay
// tren duoi dang comment, dung xoa voi.
//
// Cranelift ghim dung 0.126.0. Dung nang len neu chua doc release notes, no
// doi nhanh lam.
//
// Ca hai backend deu di vao qua def_pro, ham nay generic tren Module:
// jit.rs dua vao mot JITModule, con em_object dua vao mot ObjectModule.
// Viet ban dich mot lan thoi thi hai ben khong the tu lech nhau sau lung t.
//
// Ba cho ma Cranelift KHONG hieu giong Pump, moi cho phai no ra may lenh chu
// khong duoc mot lenh:
//
//  * dich bit. Cranelift che so dem theo do rong toan hang, nen `x << 64` ra
//  lai dung `x`. Pump thi bao dem bang
//    bit dau neu la dich phai so hoc.
//  * chia va du co dau. Cranelift trap khi int.min / -1, con Pump thi cuon ve
//    int.min voi so du 0. Nen chia cho -1 thi t doi thanh chia cho 1 roi doi
//    dau thuong.
//  * du cua so thuc. Khong co lenh frem, cung khong co libcall fmod nao het,
//  nen `float ...
//    .exe da link thi cai do den tu CRT cua MSVC, con jit.rs thi dang ky mot
//    cai shim cung ten.

use std::collections::HashMap;

use cranelift_codegen::ir as cl;
use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{types, AbiParam, InstBuilder, MemFlags, TrapCode};
use cranelift_codegen::isa::OwnedTargetIsa;
use cranelift_codegen::settings::{self, Configurable};
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Switch};
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module};

use crate::abi::{self, IrType, RuntimeFn, TypeDescriptor, VariantDescriptor};
use crate::errors::{CompileError, ErrorCode};
use crate::ir::{
  BinaryOp, BlockRef, CompareOp, ConvertOp, FuncRef, Function, Inst, InstKind, InstRef, Program,
  SigRef, Terminator, UnaryOp, Value,
};
use crate::token::Span;

// ===== bang intrinsic =====
//
// t go tay bang nay. Thu tu KHONG phai thu tu trong abi.rs va cung khong phai
// abc: no la thu tu t hay phai mo ra xem nhat luc di do bug, tu hay nhat den
// it hay nhat. May cai nay duoc khai bao truoc, con lai lay not tu
// RuntimeFn::ALL nen khong sot cai nao.
//
// Dung sap xep lai cho "gon". Doc tu tren xuong la thay ngay t hay hong cho
// nao.
const HAY_DUNG: &[RuntimeFn] = &[
  RuntimeFn::Alloc,
  RuntimeFn::Panic,
  RuntimeFn::StringNew,
  RuntimeFn::ArrayGet,
  RuntimeFn::ErrorPending,
  RuntimeFn::StringConcat,
  RuntimeFn::ArrayPush,
  RuntimeFn::PanicIndex,
  RuntimeFn::MapLookup,
  RuntimeFn::IfaceNew,
  RuntimeFn::GcCollect,
  RuntimeFn::BoxNew,
  RuntimeFn::Println,
  RuntimeFn::ErrorTake,
  RuntimeFn::ClosureNew,
  RuntimeFn::StringEq,
  RuntimeFn::AllocBuffer,
  RuntimeFn::PanicNull,
  RuntimeFn::MapSet,
  RuntimeFn::CollectionModcount,
  RuntimeFn::ArrayLen,
  RuntimeFn::ErrorSet,
  RuntimeFn::StringFromInt,
  RuntimeFn::SetAdd,
  RuntimeFn::Exit,
];

/// The C `fmod`, the one symbol outside `RuntimeFn` that generated code may
/// reference.
pub const SYMBOL_FMOD: &str = "fmod";

const TRAP_UNREACHABLE: TrapCode = TrapCode::unwrap_user(1);

const UNHANDLED_ERROR_MESSAGE: &[u8] = b"unhandled error returned from main";

fn mem2() -> MemFlags {
  MemFlags::trusted()
}

/// Settings shared by the JIT and object paths.
#[derive(Clone, Debug)]
pub struct CodegenOptions {
  pub triple: String,
  pub opt_level: &'static str,
  pub dump_clif: bool,
  pub emit_c_main: bool,
}

impl Default for CodegenOptions {
  fn default() -> CodegenOptions {
    CodegenOptions {
      triple: target_lexicon::Triple::host().to_string(),
      opt_level: "speed",
      dump_clif: false,
      emit_c_main: false,
    }
  }
}

/// The Cranelift shared flags both back ends use.
pub fn do_flags(options: &CodegenOptions) -> Result<settings::Flags, CompileError> {
  let mut builder = settings::builder();
  for (name, value) in [
    ("opt_level", options.opt_level),
    ("is_pic", "false"),
    ("use_colocated_libcalls", "false"),
  ] {
    builder.set(name, value).map_err(|error| {
      fai(format!(
        "cannot set the Cranelift flag `{name}` to `{value}`: {error}"
      ))
    })?;
  }
  Ok(settings::Flags::new(builder))
}

/// Builds the target ISA for `options.triple`.
pub fn ta_isa(options: &CodegenOptions) -> Result<OwnedTargetIsa, CompileError> {
  let unsupported = |message: String| {
    CompileError::at(ErrorCode::UnsupportedTarget, Span::synthetic(), message)
  };
  let triple: target_lexicon::Triple = options.triple.parse().map_err(|error| {
    unsupported(format!(
      "`{}` is not a target triple: {error}",
      options.triple
    ))
  })?;
  let builder = cranelift_codegen::isa::lookup(triple).map_err(|error| {
    unsupported(format!(
      "Cranelift cannot target `{}`: {error}",
      options.triple
    ))
  })?;
  builder
    .finish(do_flags(options)?)
    .map_err(|error| fai(format!("cannot configure the target: {error}")))
}

/// Translates a whole program into `module`: every function, every static,
/// and the compiler-emitted entry points of `docs/abi.md` section 7.
pub fn def_pro<M: Module>(
  module: &mut M,
  program: &Program,
  options: &CodegenOptions,
) -> Result<(), CompileError> {
  let mut codegen = Codegen::new(module, program, options)?;
  codegen.declare_imports()?;
  codegen.declare_functions()?;
  codegen.declare_statics()?;
  codegen.define_statics()?;
  codegen.define_functions()?;
  codegen.define_program_main()?;
  if options.emit_c_main {
    codegen.define_c_main()?;
  }
  Ok(())
}

/// Compiles a program to a native object file, for `pump build`.
pub fn em_object(program: &Program, options: &CodegenOptions) -> Result<Vec<u8>, CompileError> {
  let isa = ta_isa(options)?;
  let builder = cranelift_object::ObjectBuilder::new(
    isa,
    "pump".to_string(),
    cranelift_module::default_libcall_names(),
  )
  .map_err(|error| fai(format!("cannot start the object file: {error}")))?;
  let mut module = cranelift_object::ObjectModule::new(builder);

  let mut options = options.clone();
  options.emit_c_main = true;
  def_pro(&mut module, program, &options)?;

  module.finish().emit().map_err(|error| {
    CompileError::at(
      ErrorCode::ObjectEmissionFailed,
      Span::synthetic(),
      format!("cannot encode the object file: {error}"),
    )
  })
}

fn fai(message: impl Into<String>) -> CompileError {
  CompileError::at(ErrorCode::CodegenFailed, Span::synthetic(), message)
}

fn failure_at(span: Span, message: impl Into<String>) -> CompileError {
  CompileError::at(ErrorCode::CodegenFailed, span, message)
}

// === anh byte tinh ==========================
