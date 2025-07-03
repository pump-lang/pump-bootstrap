// lower. AST da ...
//
// day la cho ...
// ma backend khong phai nghi lai deu chot o day:
//
//  * mono ...
//    danh sach ban da ghi lai, va tu tung itable,
//  * layout ...
// cai nay  * type
//    ref_offsets la GC no do, nen phai dung crate::abi ma dung, tuyet doi
//    khong go tay,
//  * itable - moi conformance mot cai, method xep theo dung thu tu cua
//    interface,
//  * duong hoa - vong for,
//  nhanh re theo o loi, `?`
//    load-tinh-store, closure thanh capture cong mot con tro code,
//  * dong hop - bien bi capture thanh mot cai hop dung chung cho tat ca ai
//    capture no, va primitive optional cung thanh mot cai hop.
//
// Hai dieu tuyet doi khong duoc truot, ca hai lay tu docs/abi.md muc 22:
// khong con tro noi bo nao duoc song vat qua mot cho co the cap phat, va moi
// bien deu phai ...
// cai nay bien trong o
// dung o cho ...

use std::collections::{HashMap, HashSet};

use crate::abi::{self, DescriptorKind, IrType, RuntimeFn, TypeDescriptor, VariantDescriptor};
use crate::ast::{
    Argument, AssignStmt, Block, CatchHandler, ClosureExpr, ConstDecl, Expr, ExprKind,
    FieldPattern, ForStmt, Ident, IfStmt, IrrefutablePattern, IrrefutablePatternKind, MapEntry,
    MatchArmBody, MatchStmt, NodeId, Param, ParamKind, Pattern, PatternKind, RangeEndpoint, Stmt,
    StmtKind, StringLit, StringPart, StructLit, WhileStmt,
};
use crate::ast::{BinaryOp as SourceBinaryOp, UnaryOp as SourceUnaryOp};
use crate::check::{
    BoundArgument, BuiltinMethod, Callee, Checked, ConformanceMethod, FieldAccess, ResolvedCall,
};
use crate::errors::{CompileError, ErrorCode};
use crate::ir::{
    BinaryOp, BlockRef, CompareOp, ConvertOp, EnumSingleton, FuncRef, Function, Global, GlobalRef,
    InstKind, Itable, ItableRef, Program, Signature, SlotRef, Terminator, TypeIdx, UnaryOp, Value,
};
use crate::resolve::{GlobalConstId, LocalId, Predeclared, ValueBinding};
use crate::token::Span;
use crate::types::{
    ConstValue, DefId, FuncId, GenericOwner, ModuleId, TypeContext, TypeId, TypeKind,
};

const INSTANCE_LIMIT: usize = 20_000;

/// Lower a checked program down to IR.
pub fn lower(checked: &Checked) -> Result<Program, CompileError> {
    Lowerer::new(checked).run()
}

type InstanceKey = (FuncId, Vec<TypeId>);
