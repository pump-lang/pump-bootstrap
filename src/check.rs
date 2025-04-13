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

type Narrowings = HashMap<LocalId, TypeId>;

#[derive(Clone, Debug, Default)]
struct Facts {
  when_true: Narrowings,
  when_false: Narrowings,
}

impl Facts {
  fn inverted(self) -> Facts {
    Facts {
      when_true: self.when_false,
      when_false: self.when_true,
    }
  }
}

#[derive(Clone, Debug)]
struct Signature {
  ret: TypeId,
  failable: bool,
  this: Option<TypeId>,
  owner: Option<DefId>,
}

struct Checker<'a> {
  context: TypeContext,
  units: Rc<Vec<SourceUnit>>,
  diagnostics: &'a mut Diagnostics,

  // mang sang tu buoc resolve
  values: HashMap<NodeId, ValueBinding>,
  written_types: HashMap<NodeId, TypeId>,
  locals: Vec<LocalBinding>,
  declared_locals: HashMap<Span, LocalId>,
  closures: HashMap<NodeId, ClosureInfo>,
  functions: HashMap<FuncId, FuncLocation>,
  globals: Vec<GlobalConst>,
  implements: Vec<ImplementsAssertion>,
  pattern_defs: HashMap<NodeId, DefId>,
  const_init_order: Vec<GlobalConstId>,
  root_module: ModuleId,
  entry: Option<FuncId>,
  prelude: Prelude,

  // lam ra o day
  expression_types: HashMap<NodeId, TypeId>,
  pattern_types: HashMap<NodeId, TypeId>,
  local_types: Vec<TypeId>,
  global_types: Vec<TypeId>,
  calls: HashMap<NodeId, ResolvedCall>,
  field_accesses: HashMap<NodeId, FieldAccess>,
  constants: HashMap<NodeId, ConstValue>,
  instantiations: Vec<Instantiation>,
  seen_instantiations: HashSet<Instantiation>,
  conformances: Vec<Conformance>,
  // hai cai set nay chi de khoi
  // deu dang cam mot cai muon o cho khac roi nen t boc RefCell vao cho
  // xong chuyen. Trong day khong bao
  seen_conformances: Rc<RefCell<HashSet<(DefId, TypeId)>>>,

  // trang thai rieng cua tung than ham
  signature: Signature,
  narrowings: Narrowings,
  // 16.10 / D-25: bien nao vua bi mat thu hep vi co mot phep gan o trong
  // vung, ghi lai day kem cho gan do. Chi de bao loi cho tu te thoi, khong
  // cai nay anh huong gi
  defeated: HashMap<LocalId, Span>,
  module: ModuleId,
  closure_depth: u32,
}

impl<'a> Checker<'a> {
  fn new(
    resolution: Resolution,
    units: Rc<Vec<SourceUnit>>,
    diagnostics: &'a mut Diagnostics,
  ) -> Checker<'a> {
    let local_count = resolution.locals.len();
    let global_count = resolution.globals.len();
    Checker {
      context: resolution.context,
      units,
      diagnostics,
      values: resolution.values,
      written_types: resolution.types,
      locals: resolution.locals,
      declared_locals: resolution.declared_locals,
      closures: resolution.closures,
      functions: resolution.functions,
      globals: resolution.globals,
      implements: resolution.implements,
      pattern_defs: resolution.pattern_defs,
      const_init_order: resolution.const_init_order,
      root_module: resolution.root_module,
      entry: resolution.entry,
      prelude: resolution.prelude,
      expression_types: HashMap::new(),
      pattern_types: HashMap::new(),
      local_types: vec![TypeId::ERROR; local_count],
      global_types: vec![TypeId::ERROR; global_count],
      calls: HashMap::new(),
      field_accesses: HashMap::new(),
      constants: HashMap::new(),
      instantiations: Vec::new(),
      seen_instantiations: HashSet::new(),
      conformances: Vec::new(),
      seen_conformances: Rc::new(RefCell::new(HashSet::new())),
      signature: Signature {
        ret: TypeId::VOID,
        failable: false,
        this: None,
        owner: None,
      },
      narrowings: HashMap::new(),
      defeated: HashMap::new(),
      module: ModuleId(0),
      closure_depth: 0,
    }
  }

  fn finish(mut self) -> Checked {
    // phia sau khong duoc phep nhin thay bien suy dien, nen moi kieu da
    // ghi lai deu phai di
    let nodes: Vec<NodeId> = self.expression_types.keys().copied().collect();
    for node in nodes {
      let ty = self.expression_types[&node];
      let resolved = self.context.resolve(ty);
      self.expression_types.insert(node, resolved);
    }
    let nodes: Vec<NodeId> = self.pattern_types.keys().copied().collect();
    for node in nodes {
      let ty = self.pattern_types[&node];
      let resolved = self.context.resolve(ty);
      self.pattern_types.insert(node, resolved);
    }
    for index in 0..self.local_types.len() {
      self.local_types[index] = self.context.resolve(self.local_types[index]);
    }
    for index in 0..self.global_types.len() {
      self.global_types[index] = self.context.resolve(self.global_types[index]);
    }

    let resolution = Resolution {
      context: self.context,
      units: Vec::new(),
      root_module: self.root_module,
      entry: self.entry,
      values: self.values,
      types: self.written_types,
      locals: self.locals,
      const_init_order: self.const_init_order,
      globals: self.globals,
      declared_locals: self.declared_locals,
      closures: self.closures,
      functions: self.functions,
      implements: self.implements,
      pattern_defs: self.pattern_defs,
      prelude: self.prelude,
    };
    Checked {
      resolution,
      expression_types: self.expression_types,
      pattern_types: self.pattern_types,
      local_types: self.local_types,
      global_types: self.global_types,
      calls: self.calls,
      field_accesses: self.field_accesses,
      constants: self.constants,
      instantiations: self.instantiations,
      conformances: self.conformances,
    }
  }

  fn report(&mut self, error: CompileError) {
    self.diagnostics.push(error);
  }

  fn show(&self, ty: TypeId) -> String {
    self.context.display(ty)
  }

  fn record(&mut self, node: NodeId, ty: TypeId) -> TypeId {
    self.expression_types.insert(node, ty);
    ty
  }

  fn local_type(&self, local: LocalId) -> TypeId {
    self.narrowings
      .get(&local)
      .copied()
      .unwrap_or_else(|| self.local_types[local.index()])
  }
}
