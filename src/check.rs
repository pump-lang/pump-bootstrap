// kiem tra kieu, suy dien, vet het nhanh, va moi luat ma parser tu no
// khong ep duoc.
//
// kiem tra theo hai chieu. Bieu thuc nao co kieu mong doi thi kiem theo kieu
// do, va do la ca cai meo: nho no ma so nguyen thanh uint duoc, `null` thanh
// dung mot optional cu the duoc, `{}` thanh dung mot map cu the duoc, ma
// khong cai nao ...
// cai nay roi ve kieu
// thi bao loi.
//
// may luat ngon code nhat o day:
//
//  * tuyet doi khong tu dong doi kieu, int sang float cung khong,
// cai nay  * `t!`
//  ngay tai cho
//  * than ...
// cai nay  * thu
//    field, va chi can mot phep gan trong vung do la mat thu hep ca ham,
//  * khong co truthiness, `if x` voi x khong phai bool la loi, kem goi y,
//  * interface khop theo cau truc: so tham so, kieu tung tham so theo dung
// cai nay  thu tu,
//
// cai nay generic thi mono
// nhung ban nao.
//
// File nay dai qua, t biet. Cat ra thi lai phai chia may cai bang trang thai
// ra theo, ma lam the borrow checker no lai keu.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::ast::{
  Argument, AssignStmt, BinaryOp, Block, CatchHandler, ClosureExpr, ConstDecl, Declaration,
  ElseBranch, Expr, ExprKind, FunctionDecl, Ident, IfStmt, IrrefutablePattern,
  IrrefutablePatternKind, MatchArmBody, MatchStmt, NodeId, Pattern, PatternKind, RangeEndpoint,
  SourceUnit, Stmt, StmtKind, StringPart, StructMember, TypeExpr, UnaryOp, VisibilityKind,
};
use crate::errors::{CompileError, Diagnostics, ErrorCode};
use crate::resolve::{
  ClosureInfo, FuncLocation, GlobalConst, GlobalConstId, ImplementsAssertion, LocalBinding,
  LocalId, LocalOrigin, Predeclared, Prelude, Resolution, ValueBinding,
};
use crate::token::Span;
use crate::types::{
  ConstValue, DefId, FnType, FuncId, GenericOwner, ModuleId, TypeContext, TypeId, TypeKind,
  TypeVar,
};

/// What the checker gives to the lowering pass.
#[derive(Debug)]
pub struct Checked {
  pub resolution: Resolution,
  pub expression_types: HashMap<NodeId, TypeId>,
  pub pattern_types: HashMap<NodeId, TypeId>,
  pub local_types: Vec<TypeId>,
  pub global_types: Vec<TypeId>,
  pub calls: HashMap<NodeId, ResolvedCall>,
  pub field_accesses: HashMap<NodeId, FieldAccess>,
  pub constants: HashMap<NodeId, ConstValue>,
  pub instantiations: Vec<Instantiation>,
  pub conformances: Vec<Conformance>,
}

/// Mot cho goi ham, sau khi da buoc doi so xong. 13.6.2.
#[derive(Clone, Debug)]
pub struct ResolvedCall {
  pub callee: Callee,
  pub arguments: Vec<BoundArgument>,
  pub type_arguments: Vec<TypeId>,
  pub failable: bool,
}
