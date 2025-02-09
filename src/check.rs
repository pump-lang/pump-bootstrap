// kiem tra kieu, suy dien, vet het nhanh, va moi luat ma parser tu no
// khong ep duoc.
//
// kiem tra theo hai chieu. Bieu thuc nao co kieu mong doi thi kiem theo kieu
// do, va do la ca cai meo: nho no ma so nguyen thanh uint duoc, `null` thanh
// dung mot optional cu the duoc, `{}` thanh dung mot map cu the duoc, ma
// khong cai nao can co kieu cua rieng no. Khong co mong doi gi thi bieu thuc
// roi ve kieu tu nhien cua no, con collection rong ma khong co gi de suy ra
// thi bao loi.
//
// may luat ngon code nhat o day:
//
//  * tuyet doi khong tu dong doi kieu, int sang float cung khong,
//  * `T!` chi duoc dat o kieu tra ve, va loi goi co the that bai thi phai an
//    ngay tai cho bang `!` hoac bang catch,
//  * than cua catch bat buoc phai phan ky,
//  * thu hep null chi lam tren ten bien don, khong bao gio tren duong dan
//    field, va chi can mot phep gan trong vung do la mat thu hep ca ham,
//  * khong co truthiness, `if x` voi x khong phai bool la loi, kem goi y,
//  * interface khop theo cau truc: so tham so, kieu tung tham so theo dung
//    thu tu, va kieu tra ve. Ten tham so va gia tri mac dinh khong tinh.
//
// generic thi mono o pha sau, nen o day chi ghi lai xem lower se phai sinh
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

/// Cai ma mot cho goi ham nhay toi.
#[derive(Clone, Debug)]
pub enum Callee {
  Function(FuncId),
  Method { owner: DefId, func: FuncId },
  Interface { interface: DefId, slot: u32 },
  Closure,
  Conversion { target: TypeId },
  Predeclared(Predeclared),
  Builtin(BuiltinMethod),
  Variant { def: DefId, variant: u32 },
}

/// One argument, sau khi da dien theo vi tri, gom variadic, va the gia tri
/// mac dinh vao.
#[derive(Clone, Debug)]
pub enum BoundArgument {
  Receiver(NodeId),
  Expression(NodeId),
  Default(ConstValue),
  Variadic(Vec<NodeId>),
}

/// Mot ban generic ma lower se phai sinh ra.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Instantiation {
  pub func: FuncId,
  pub type_arguments: Vec<TypeId>,
}

/// Mot cap (interface, kieu cu the) can mot itable.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Conformance {
  pub interface: DefId,
  pub concrete: TypeId,
  pub methods: Vec<ConformanceMethod>,
}

/// One slot of an itable.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ConformanceMethod {
  User(FuncId),
  Builtin(BuiltinMethod),
}

/// Cai ma mot bieu thuc `.name` cham toi.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FieldAccess {
  Field { owner: DefId, index: u32 },
  Length,
}

/// Method co san. Tam thoi no dong vai cai stdlib be ti cua Pump 1.0.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BuiltinMethod {
  ToString,
  StringMessage,
  StringChars,
  StringCharCount,
  StringByteAt,
  StringSlice,
  ArrayPush,
  ArrayPop,
  ArraySlice,
  ArrayConcat,
  ArrayReserve,
  MapHas,
  MapGet,
  MapInsert,
  MapRemove,
  MapKeys,
  MapValues,
  SetAdd,
  SetHas,
  SetRemove,
  OptionalExpect,
  OptionalOr,
}

impl BuiltinMethod {
  pub fn spelling(self) -> &'static str {
    use BuiltinMethod::*;
    match self {
      ToString => "to_string",
      StringMessage => "message",
      StringChars => "chars",
      StringCharCount => "char_count",
      StringByteAt => "byte_at",
      StringSlice => "slice",
      ArrayPush => "push",
      ArrayPop => "pop",
      ArraySlice => "slice",
      ArrayConcat => "concat",
      ArrayReserve => "reserve",
      MapHas => "has",
      MapGet => "get",
      MapInsert => "insert",
      MapRemove => "remove",
      MapKeys => "keys",
      MapValues => "values",
      SetAdd => "add",
      SetHas => "has",
      SetRemove => "remove",
      OptionalExpect => "expect",
      OptionalOr => "or",
    }
  }
}

impl Checked {
  pub fn context(&self) -> &TypeContext {
    &self.resolution.context
  }

  pub fn context_mut(&mut self) -> &mut TypeContext {
    &mut self.resolution.context
  }

  /// Kieu da ghi cho mot bieu thuc. TypeId::ERROR neu chua bao gio toi
  /// duoc cho do vi mot loi truoc da chan lai.
  pub fn type_of(&self, node: NodeId) -> TypeId {
    self.expression_types
      .get(&node)
      .copied()
      .unwrap_or(TypeId::ERROR)
  }
}

/// Check a program that resolve already went over.
pub fn check(
  mut resolution: Resolution,
  diagnostics: &mut Diagnostics,
) -> Result<Checked, CompileError> {
  let units = Rc::new(std::mem::take(&mut resolution.units));
  let mut checker = Checker::new(resolution, Rc::clone(&units), diagnostics);

  checker.check_module_constants();
  checker.check_functions();
  checker.check_implements();

  let mut checked = checker.finish();
  checked.resolution.units =
    Rc::try_unwrap(units).unwrap_or_else(|shared| shared.as_ref().clone());
  Ok(checked)
}
