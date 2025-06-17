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
  // hai cai set nay chi de khoi ghi trung. Cho nao goi den chung thi
  // deu dang cam mot cai muon o cho khac roi nen t boc RefCell vao cho
  // xong chuyen. Trong day khong bao gio muon long nhau nen khong no.
  seen_conformances: Rc<RefCell<HashSet<(DefId, TypeId)>>>,

  // trang thai rieng cua tung than ham
  signature: Signature,
  narrowings: Narrowings,
  // 16.10 / D-25: bien nao vua bi mat thu hep vi co mot phep gan o trong
  // vung, ghi lai day kem cho gan do. Chi de bao loi cho tu te thoi, khong
  // anh huong gi den viec kiem tra ca.
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
    // ghi lai deu phai di qua substitution mot lan cuoi
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

impl Checker<'_> {
  fn unify(&mut self, left: TypeId, right: TypeId) -> bool {
    let left = self.context.shallow_resolve(left);
    let right = self.context.shallow_resolve(right);
    if left == right {
      return true;
    }
    let left_kind = self.context.kind(left).clone();
    let right_kind = self.context.kind(right).clone();
    match (left_kind, right_kind) {
      (TypeKind::Error, _) | (_, TypeKind::Error) => true,
      (TypeKind::Var(var), _) => self.bind(var, right),
      (_, TypeKind::Var(var)) => self.bind(var, left),
      (TypeKind::Array(a), TypeKind::Array(b)) => self.unify(a, b),
      (TypeKind::Set(a), TypeKind::Set(b)) => self.unify(a, b),
      (TypeKind::Map { key: ka, value: va }, TypeKind::Map { key: kb, value: vb }) => {
        self.unify(ka, kb) && self.unify(va, vb)
      }
      (TypeKind::Tuple(a), TypeKind::Tuple(b)) if a.len() == b.len() => {
        a.iter().zip(b.iter()).all(|(&x, &y)| self.unify(x, y))
      }
      (TypeKind::Optional(a), TypeKind::Optional(b)) => self.unify(a, b),
      (TypeKind::Failable(a), TypeKind::Failable(b)) => self.unify(a, b),
      (TypeKind::Function(a), TypeKind::Function(b)) => {
        a.params.len() == b.params.len()
          && a.failable == b.failable
          && a.variadic.is_some() == b.variadic.is_some()
          && a.params
            .iter()
            .zip(b.params.iter())
            .all(|(&x, &y)| self.unify(x, y))
          && match (a.variadic, b.variadic) {
            (Some(x), Some(y)) => self.unify(x, y),
            _ => true,
          }
          && self.unify(a.ret, b.ret)
      }
      (TypeKind::Named { def: da, args: aa }, TypeKind::Named { def: db, args: ab })
        if da == db && aa.len() == ab.len() =>
      {
        aa.iter().zip(ab.iter()).all(|(&x, &y)| self.unify(x, y))
      }
      _ => false,
    }
  }

  fn bind(&mut self, var: TypeVar, ty: TypeId) -> bool {
    if self.context.occurs_in(var, ty) {
      return false;
    }
    if self.context.var_binding(var).is_some() {
      let bound = self
        .context
        .var_binding(var)
        .expect("just observed as bound");
      return self.unify(bound, ty);
    }
    self.context.bind_var(var, ty);
    true
  }

  fn assignable(&mut self, target: TypeId, value: TypeId) -> bool {
    let target = self.context.shallow_resolve(target);
    let value = self.context.shallow_resolve(value);
    if target == value {
      return true;
    }
    if matches!(self.context.kind(value), TypeKind::Never)
      || matches!(self.context.kind(target), TypeKind::Never)
    {
      return true;
    }
    if self.unify(target, value) {
      return true;
    }
    if let TypeKind::Optional(inner) = *self.context.kind(target) {
      if self.assignable(inner, value) {
        return true;
      }
    }
    if let TypeKind::Named { def, .. } = *self.context.kind(target) {
      if self.context.def(def).as_interface().is_some() {
        return self.record_conformance(def, value).is_some();
      }
    }
    false
  }

  fn expect_assignable(&mut self, target: TypeId, value: TypeId, span: Span, context: &str) {
    if self.assignable(target, value) {
      return;
    }
    let expected = self.show(target);
    let found = self.show(value);
    let mut error = CompileError::at(
      ErrorCode::TypeMismatch,
      span,
      format!("{context} expects `{expected}`, but this is `{found}`"),
    )
    .with_caret(format!("this is `{found}`"));

    if self.context.is_numeric(target) && self.context.is_numeric(value) {
      let call = match *self.context.kind(target) {
        TypeKind::Int => "int",
        TypeKind::Uint => "uint",
        TypeKind::Float => "float",
        _ => "",
      };
      error = error
        .with_note("Pump has no implicit numeric conversions, in either direction")
        .with_help(format!("write the conversion: `{call}(x)`"));
    } else if matches!(self.context.kind(value), TypeKind::Optional(_))
      && !matches!(self.context.kind(target), TypeKind::Optional(_))
    {
      error = error.with_help(
        "an optional is not its inner type; test it with `!= null`, or unwrap it with `?`",
      );
    }
    self.report(error);
  }
}

impl Checker<'_> {
  fn record_conformance(
    &mut self,
    interface: DefId,
    concrete: TypeId,
  ) -> Option<Vec<ConformanceMethod>> {
    let concrete = self.context.resolve(concrete);
    if let TypeKind::Named { def, .. } = *self.context.kind(concrete) {
      if def == interface {
        // gia tri interface tu no da mang itable roi
        return Some(Vec::new());
      }
    }
    let methods = self.conformance_methods(interface, concrete)?;
    let moi = self
      .seen_conformances
      .borrow_mut()
      .insert((interface, concrete));
    if moi {
      self.conformances.push(Conformance {
        interface,
        concrete,
        methods: methods.clone(),
      });
    }
    Some(methods)
  }

  fn conformance_methods(
    &mut self,
    interface: DefId,
    concrete: TypeId,
  ) -> Option<Vec<ConformanceMethod>> {
    if let Some(builtin) = self.primitive_conformance(interface, concrete) {
      return Some(builtin);
    }
    let TypeKind::Named { def, args } = self.context.kind(concrete).clone() else {
      return None;
    };
    if self.context.def(def).as_interface().is_some() {
      // doi interface sang interface thi can embedding, 1.0 chua co
      return None;
    }
    let slots = self.context.def(interface).methods().to_vec();
    let mut out = Vec::with_capacity(slots.len());
    for slot in slots {
      let name = self.context.func(slot).name.clone();
      let candidate = self.context.find_method(def, &name)?;
      if !self.signatures_match(slot, candidate, &args, def) {
        return None;
      }
      out.push(ConformanceMethod::User(candidate));
    }
    Some(out)
  }

  fn primitive_conformance(
    &self,
    interface: DefId,
    concrete: TypeId,
  ) -> Option<Vec<ConformanceMethod>> {
    let primitive = matches!(
      self.context.kind(concrete),
      TypeKind::Bool
        | TypeKind::Int
        | TypeKind::Uint
        | TypeKind::Float
        | TypeKind::Char
        | TypeKind::String
    );
    if !primitive {
      return None;
    }
    if interface == self.prelude.stringable {
      return Some(vec![ConformanceMethod::Builtin(BuiltinMethod::ToString)]);
    }
    if interface == self.prelude.error && concrete == TypeId::STRING {
      return Some(vec![ConformanceMethod::Builtin(
        BuiltinMethod::StringMessage,
      )]);
    }
    None
  }

  fn signatures_match(
    &mut self,
    expected: FuncId,
    candidate: FuncId,
    concrete_args: &[TypeId],
    concrete_def: DefId,
  ) -> bool {
    let wanted = self.context.func(expected).clone();
    let found = self.context.func(candidate).clone();
    if wanted.params.len() != found.params.len()
      || wanted.failable != found.failable
      || wanted.generics.len() != found.generics.len()
      || wanted.variadic_index() != found.variadic_index()
    {
      return false;
    }

    for index in 0..wanted.params.len() {
      let left = self.instantiate_interface_type(wanted.params[index].ty, expected);
      let right = self.instantiate_concrete_type(
        found.params[index].ty,
        candidate,
        expected,
        concrete_def,
        concrete_args,
      );
      if self.context.resolve(left) != self.context.resolve(right) {
        return false;
      }
    }
    let left = self.instantiate_interface_type(wanted.ret, expected);
    let right = self.instantiate_concrete_type(
      found.ret,
      candidate,
      expected,
      concrete_def,
      concrete_args,
    );
    self.context.resolve(left) == self.context.resolve(right)
  }

  fn instantiate_interface_type(&mut self, ty: TypeId, _method: FuncId) -> TypeId {
    ty
  }

  fn instantiate_concrete_type(
    &mut self,
    ty: TypeId,
    candidate: FuncId,
    expected: FuncId,
    concrete_def: DefId,
    concrete_args: &[TypeId],
  ) -> TypeId {
    let ty = self
      .context
      .substitute(ty, GenericOwner::Type(concrete_def), concrete_args);
    let count = self.context.func(candidate).generics.len();
    if count == 0 {
      return ty;
    }
    let mapping: Vec<TypeId> = (0..count)
      .map(|index| {
        self.context
          .intern(TypeKind::Generic(crate::types::GenericId {
            owner: GenericOwner::Func(expected),
            index: index as u32,
          }))
      })
      .collect();
    self.context
      .substitute(ty, GenericOwner::Func(candidate), &mapping)
  }

  fn check_generic_bounds(
    &mut self,
    owner: GenericOwner,
    arguments: &[TypeId],
    span: Span,
  ) -> bool {
    let parameters = match owner {
      GenericOwner::Type(def) => self.context.def(def).generics.clone(),
      GenericOwner::Func(func) => self.context.func(func).generics.clone(),
    };
    let mut satisfied = true;

    for (parameter, &argument) in parameters.iter().zip(arguments) {
      if parameter.bounds.is_empty() {
        continue;
      }
      let argument = self.context.resolve(argument);
      if matches!(
        self.context.kind(argument),
        TypeKind::Generic(_) | TypeKind::Error
      ) || self.has_inference_variable(argument)
      {
        continue;
      }

      for &interface in &parameter.bounds {
        if self.record_conformance(interface, argument).is_some() {
          continue;
        }
        let shown = self.show(argument);
        let name = self.context.def(interface).name.clone();
        let bound_on = parameter.name.clone();
        let missing = self.describe_missing_methods(interface, argument);
        let mut error = CompileError::at(
          ErrorCode::InterfaceNotSatisfied,
          span,
          format!("`{shown}` does not satisfy `{name}`, the bound on `{bound_on}`"),
        )
        .with_secondary(parameter.span, "the bound is declared here");
        for line in missing {
          error = error.with_note(line);
        }
        self.report(error);
        satisfied = false;
      }
    }

    satisfied
  }

  fn check_implements(&mut self) {
    let assertions = std::mem::take(&mut self.implements);
    for assertion in &assertions {
      let concrete = self.context.named(assertion.subject, Vec::new());
      for &(interface, span) in &assertion.interfaces {
        if self.record_conformance(interface, concrete).is_some() {
          continue;
        }
        let subject = self.context.def(assertion.subject).name.clone();
        let name = self.context.def(interface).name.clone();
        let missing = self.describe_missing_methods(interface, concrete);
        let mut error = CompileError::at(
          ErrorCode::InterfaceNotSatisfied,
          span,
          format!("`{subject}` does not satisfy `{name}`"),
        )
        .with_secondary(assertion.subject_span, "this type");
        for line in missing {
          error = error.with_note(line);
        }
        self.report(error);
      }
    }
    self.implements = assertions;
  }

  fn describe_missing_methods(&mut self, interface: DefId, concrete: TypeId) -> Vec<String> {
    let TypeKind::Named { def, args } = self.context.kind(concrete).clone() else {
      return vec![format!(
        "`{}` is not a named type, so it declares no methods",
        self.show(concrete)
      )];
    };
    let slots = self.context.def(interface).methods().to_vec();
    let mut out = Vec::new();
    for slot in slots {
      let name = self.context.func(slot).name.clone();
      match self.context.find_method(def, &name) {
        None => out.push(format!(
          "missing method `{}`",
          self.describe_signature(slot)
        )),
        Some(candidate) => {
          if !self.signatures_match(slot, candidate, &args, def) {
            let wanted = self.describe_signature(slot);
            let found = self.describe_signature(candidate);
            out.push(format!("`{name}` is `{found}` but must be `{wanted}`"));
          }
        }
      }
    }
    out
  }

  fn describe_signature(&self, func: FuncId) -> String {
    let definition = self.context.func(func);
    let mut out = format!("fn {}(", definition.name);
    for (index, param) in definition.params.iter().enumerate() {
      if index > 0 {
        out.push_str(", ");
      }
      if param.variadic {
        out.push_str("...");
      }
      out.push_str(&self.show(param.ty));
    }
    out.push(')');
    if definition.ret != TypeId::VOID || definition.failable {
      out.push_str(": ");
      out.push_str(&self.show(definition.ret));
    }
    if definition.failable {
      out.push('!');
    }
    out
  }
}

impl Checker<'_> {
  fn check_module_constants(&mut self) {
    let units = Rc::clone(&self.units);
    let order: Vec<GlobalConstId> = self.const_init_order.clone();
    let mut done: HashSet<usize> = HashSet::new();

    for global in order {
      let entry = self.globals[global.index()].clone();
      if !done.insert(entry.declaration * 128 + entry.module.index()) {
        continue;
      }
      let Some(Declaration::Const(decl)) = units[entry.module.index()]
        .declarations
        .get(entry.declaration)
      else {
        continue;
      };
      self.module = entry.module;
      self.signature = Signature {
        ret: TypeId::VOID,
        failable: false,
        this: None,
        owner: None,
      };
      self.narrowings.clear();
      self.defeated.clear();
      self.check_const_declaration(decl, entry.module);
    }
  }

  fn check_const_declaration(&mut self, decl: &ConstDecl, module: ModuleId) {
    let annotation = decl
      .ty
      .as_ref()
      .and_then(|written| self.written_types.get(&written.id).copied());
    let value = self.check_expr(&decl.value, annotation);
    let value = self.demand_value(value, decl.value.span);
    let bound = annotation.unwrap_or(value);
    if let Some(annotation) = annotation {
      self.expect_assignable(annotation, value, decl.value.span, "this constant");
    }
    self.bind_module_pattern(&decl.pattern, bound, module);
  }

  fn bind_module_pattern(&mut self, pattern: &IrrefutablePattern, ty: TypeId, module: ModuleId) {
    match &pattern.kind {
      IrrefutablePatternKind::Wildcard => {}
      IrrefutablePatternKind::Binding(name) => {
        let entry = self.globals.iter().find(|global| {
          global.module == module && global.name == name.name && global.span == name.span
        });
        if let Some(entry) = entry {
          let index = entry.id.index();
          self.global_types[index] = ty;
        }
      }
      IrrefutablePatternKind::Tuple(elements) => {
        let parts = match self.context.kind(self.context.shallow_resolve(ty)).clone() {
          TypeKind::Tuple(parts) if parts.len() == elements.len() => parts,
          _ => {
            self.report_destructure_mismatch(pattern, ty, elements.len());
            vec![TypeId::ERROR; elements.len()]
          }
        };
        for (element, part) in elements.iter().zip(parts) {
          self.bind_module_pattern(element, part, module);
        }
      }
    }
  }

  fn report_destructure_mismatch(
    &mut self,
    pattern: &IrrefutablePattern,
    ty: TypeId,
    arity: usize,
  ) {
    let shown = self.show(ty);
    self.report(
      CompileError::at(
        ErrorCode::TypeMismatch,
        pattern.span,
        format!("cannot destructure `{shown}` into {arity} bindings"),
      )
      .with_help("the right-hand side must be a tuple of the same length"),
    );
  }

  fn check_functions(&mut self) {
    let units = Rc::clone(&self.units);
    let mut ordered: Vec<(FuncId, FuncLocation)> =
      self.functions.iter().map(|(&id, &at)| (id, at)).collect();
    ordered.sort_by_key(|(id, _)| *id);

    for (func, location) in ordered {
      let Some(decl) = declaration_at(&units, location) else {
        continue;
      };
      self.check_function_body(func, location.module, decl);
    }
  }

  fn check_function_body(&mut self, func: FuncId, module: ModuleId, decl: &FunctionDecl) {
    let definition = self.context.func(func).clone();
    let this = definition.owner.map(|owner| self.self_type(owner));

    self.module = module;
    self.signature = Signature {
      ret: definition.ret,
      failable: definition.failable,
      this,
      owner: definition.owner,
    };
    self.narrowings.clear();
    self.defeated.clear();
    self.closure_depth = 0;

    for param in &decl.params {
      let Some(index) = definition.param_index(&param.name.name) else {
        continue;
      };
      let declared = &definition.params[index];
      let ty = if declared.variadic {
        self.context.array_of(declared.ty)
      } else {
        declared.ty
      };
      self.bind_local(&param.name, ty);
      if let Some(default) = declared.default.clone() {
        self.check_default_value(&default, declared.ty, param.span);
      }
    }

    let diverges = self.check_block(&decl.body);
    if definition.ret != TypeId::VOID && !diverges {
      let shown = self.show(definition.ret);
      self.report(
        CompileError::at(
          ErrorCode::MissingReturn,
          decl.body.span,
          format!("`{}` must return `{shown}` on every path", decl.name.name),
        )
        .with_caret("this block can finish without returning")
        .with_note("Pump has no implicit return; a trailing expression is not a value"),
      );
    }
  }

  fn self_type(&mut self, owner: DefId) -> TypeId {
    let count = self.context.def(owner).generics.len();
    let args: Vec<TypeId> = (0..count)
      .map(|index| {
        self.context
          .intern(TypeKind::Generic(crate::types::GenericId {
            owner: GenericOwner::Type(owner),
            index: index as u32,
          }))
      })
      .collect();
    self.context.named(owner, args)
  }

  fn check_default_value(&mut self, value: &ConstValue, ty: TypeId, span: Span) {
    let actual = self.type_of_constant(value);
    let Some(actual) = actual else { return };
    if !self.assignable(ty, actual) {
      let expected = self.show(ty);
      let found = self.show(actual);
      self.report(CompileError::at(
        ErrorCode::TypeMismatch,
        span,
        format!("the default is `{found}` but the parameter is `{expected}`"),
      ));
    }
  }

  fn type_of_constant(&mut self, value: &ConstValue) -> Option<TypeId> {
    Some(match value {
      ConstValue::Bool(_) => TypeId::BOOL,
      ConstValue::Int(_) => TypeId::INT,
      ConstValue::Uint(_) => TypeId::UINT,
      ConstValue::Float(_) => TypeId::FLOAT,
      ConstValue::Char(_) => TypeId::CHAR,
      ConstValue::Str(_) => TypeId::STRING,
      ConstValue::Null => return None,
      ConstValue::Array(elements) => {
        let element = self.type_of_constant(elements.first()?)?;
        self.context.array_of(element)
      }
      ConstValue::Set(elements) => {
        let element = self.type_of_constant(elements.first()?)?;
        self.context.set_of(element)
      }
      ConstValue::Map(entries) => {
        let (key, value) = entries.first()?;
        let key = self.type_of_constant(key)?;
        let value = self.type_of_constant(value)?;
        self.context.map_of(key, value)
      }
      ConstValue::Tuple(elements) => {
        let mut parts = Vec::with_capacity(elements.len());
        for element in elements {
          parts.push(self.type_of_constant(element)?);
        }
        self.context.tuple_of(parts)
      }
      ConstValue::EnumVariant { def, .. } => self.self_type(*def),
    })
  }

  fn bind_local(&mut self, name: &Ident, ty: TypeId) -> Option<LocalId> {
    let local = self.declared_locals.get(&name.span).copied()?;
    self.local_types[local.index()] = ty;
    self.narrowings.remove(&local);
    Some(local)
  }
}

fn declaration_at(units: &[SourceUnit], location: FuncLocation) -> Option<&FunctionDecl> {
  let unit = units.get(location.module.index())?;
  match (
    unit.declarations.get(location.declaration)?,
    location.member,
  ) {
    (Declaration::Function(decl), None) => Some(decl),
    (Declaration::Struct(decl), Some(index)) => match decl.members.get(index)? {
      StructMember::Method(method) => Some(method),
      StructMember::Field(_) => None,
    },
    (Declaration::Enum(decl), Some(index)) => match decl.members.get(index)? {
      crate::ast::EnumMember::Method(method) => Some(method),
      crate::ast::EnumMember::Variant(_) => None,
    },
    _ => None,
  }
}

impl Checker<'_> {
  fn check_block(&mut self, block: &Block) -> bool {
    let saved = self.narrowings.clone();
    let mut diverges = false;
    for (index, statement) in block.statements.iter().enumerate() {
      let rest = &block.statements[index + 1..];
      if self.check_stmt(statement, rest) {
        diverges = true;
      }
    }
    self.narrowings = saved;
    diverges
  }

  fn check_stmt(&mut self, statement: &Stmt, rest: &[Stmt]) -> bool {
    match &statement.kind {
      StmtKind::Let(decl) => {
        let annotation = self.written_annotation(decl.ty.as_ref());
        let value = self.check_expr(&decl.value, annotation);
        let value = self.demand_value(value, decl.value.span);
        if let Some(annotation) = annotation {
          self.expect_assignable(annotation, value, decl.value.span, "this binding");
        }
        let bound = annotation.unwrap_or(value);
        self.bind_irrefutable(&decl.pattern, bound);
        false
      }
      StmtKind::Const(decl) => {
        let annotation = self.written_annotation(decl.ty.as_ref());
        let value = self.check_expr(&decl.value, annotation);
        let value = self.demand_value(value, decl.value.span);
        if let Some(annotation) = annotation {
          self.expect_assignable(annotation, value, decl.value.span, "this binding");
        }
        let bound = annotation.unwrap_or(value);
        self.bind_irrefutable(&decl.pattern, bound);
        false
      }
      StmtKind::Assign(assign) => {
        self.check_assignment(assign);
        false
      }
      StmtKind::Expr(expr) => {
        let ty = self.check_expr(expr, None);
        let ty = self.demand_value(ty, expr.span);
        matches!(self.context.kind(ty), TypeKind::Never)
      }
      StmtKind::If(statement) => self.check_if(statement, rest),
      StmtKind::While(statement) => {
        let facts = self.check_condition(&statement.condition);
        let saved = self.enter_narrowed(&facts.when_true, &statement.body.statements);
        self.check_block(&statement.body);
        self.narrowings = saved;
        // `while true` khong co duong ra la vong lap duy nhat phan ky
        let endless = matches!(statement.condition.kind, ExprKind::Bool(true));
        endless && !contains_break(&statement.body)
      }
      StmtKind::For(statement) => {
        let iterable = self.check_expr(&statement.iterable, None);
        let iterable = self.demand_value(iterable, statement.iterable.span);
        let element = self.element_type(iterable, statement.iterable.span);
        self.bind_irrefutable(&statement.pattern, element);
        self.check_block(&statement.body);
        false
      }
      StmtKind::Match(statement) => self.check_match(statement),
      StmtKind::Return(value) => {
        self.check_return(value.as_ref(), statement.span);
        true
      }
      StmtKind::Fail(value) => {
        self.check_fail(value, statement.span);
        true
      }
      StmtKind::Break | StmtKind::Continue => true,
      StmtKind::Block(block) => self.check_block(block),
    }
  }

  fn written_annotation(&self, written: Option<&TypeExpr>) -> Option<TypeId> {
    written.and_then(|written| self.written_types.get(&written.id).copied())
  }

  fn check_if(&mut self, statement: &IfStmt, rest: &[Stmt]) -> bool {
    let facts = self.check_condition(&statement.condition);

    let saved = self.enter_narrowed(&facts.when_true, &statement.then_block.statements);
    let then_diverges = self.check_block(&statement.then_block);
    self.narrowings = saved;

    let else_diverges = match &statement.else_branch {
      None => false,
      Some(ElseBranch::Block(block)) => {
        let saved = self.enter_narrowed(&facts.when_false, &block.statements);
        let diverges = self.check_block(block);
        self.narrowings = saved;
        diverges
      }
      Some(ElseBranch::If(nested)) => {
        let saved = self.enter_narrowed(
          &facts.when_false,
          std::slice::from_ref(&Stmt {
            id: NodeId::NONE,
            kind: StmtKind::If(nested.as_ref().clone()),
            span: nested.span,
          }),
        );
        let diverges = self.check_if(nested, &[]);
        self.narrowings = saved;
        diverges
      }
    };

    // `if x == null { return }` thu hep x cho tat ca phan sau do
    if then_diverges && statement.else_branch.is_none() && !facts.when_false.is_empty() {
      let keep = self.surviving(&facts.when_false, rest);
      self.narrowings.extend(keep);
    }
    then_diverges && else_diverges
  }

  fn enter_narrowed(&mut self, facts: &Narrowings, region: &[Stmt]) -> Narrowings {
    let saved = self.narrowings.clone();
    let keep = self.surviving(facts, region);
    self.narrowings.extend(keep);
    saved
  }

  fn surviving(&mut self, facts: &Narrowings, region: &[Stmt]) -> Narrowings {
    let mut keep = Narrowings::new();
    for (&local, &ty) in facts {
      match statements_assign(&self.values, region, local) {
        Some(at) => {
          self.defeated.insert(local, at);
        }
        None => {
          keep.insert(local, ty);
        }
      }
    }
    keep
  }

  fn check_return(&mut self, value: Option<&Expr>, span: Span) {
    let expected = self.signature.ret;
    match value {
      None => {
        if expected != TypeId::VOID {
          let shown = self.show(expected);
          self.report(
            CompileError::at(
              ErrorCode::MissingReturnValue,
              span,
              format!("this function returns `{shown}`"),
            )
            .with_help(format!("write `return <{shown}>`")),
          );
        }
      }
      Some(value) => {
        let actual = self.check_expr(value, Some(expected));
        let actual = self.demand_value(actual, value.span);
        if expected == TypeId::VOID {
          self.report(
            CompileError::at(
              ErrorCode::ReturnValueInVoidFunction,
              value.span,
              "this function has no return type, so it cannot return a value",
            )
            .with_help("declare a return type with `: T`"),
          );
        } else {
          self.expect_assignable(expected, actual, value.span, "this `return`");
        }
      }
    }
  }

  fn check_fail(&mut self, value: &Expr, span: Span) {
    if !self.signature.failable {
      self.report(
        CompileError::at(
          ErrorCode::FailOutsideFailable,
          span,
          "`fail` needs an enclosing function whose return type carries `!`",
        )
        .with_help("declare the function as `fn f(): T!`"),
      );
    }
    let error_type = self.prelude.error_type;
    let actual = self.check_expr(value, Some(error_type));
    let actual = self.demand_value(actual, value.span);
    if !self.assignable(error_type, actual) {
      let shown = self.show(actual);
      self.report(
        CompileError::at(
          ErrorCode::InterfaceNotSatisfied,
          value.span,
          format!("`{shown}` does not satisfy `Error`"),
        )
        .with_note("`Error` needs `fn message(): string`; `string` satisfies it already"),
      );
    }
  }

  fn element_type(&mut self, iterable: TypeId, span: Span) -> TypeId {
    let iterable = self.context.shallow_resolve(iterable);
    match self.context.kind(iterable).clone() {
      TypeKind::Error => TypeId::ERROR,
      TypeKind::Array(element) | TypeKind::Set(element) => element,
      TypeKind::Map { key, value } => self.context.tuple_of(vec![key, value]),
      TypeKind::String => TypeId::CHAR,
      TypeKind::Named { def, .. } if def == self.prelude.range => TypeId::INT,
      _ => {
        let shown = self.show(iterable);
        let mut error = CompileError::at(
          ErrorCode::NotIterable,
          span,
          format!("`{shown}` cannot be iterated"),
        )
        .with_note(
          "Pump 1.0 iterates arrays, maps, sets, strings and ranges; user-defined \
           iterables are deferred",
        );
        if matches!(self.context.kind(iterable), TypeKind::Optional(_)) {
          error = error.with_help("test it against `null` first");
        }
        self.report(error);
        TypeId::ERROR
      }
    }
  }

  fn bind_irrefutable(&mut self, pattern: &IrrefutablePattern, ty: TypeId) {
    match &pattern.kind {
      IrrefutablePatternKind::Wildcard => {}
      IrrefutablePatternKind::Binding(name) => {
        self.bind_local(name, ty);
      }
      IrrefutablePatternKind::Tuple(elements) => {
        let resolved = self.context.shallow_resolve(ty);
        let parts = match self.context.kind(resolved).clone() {
          TypeKind::Tuple(parts) if parts.len() == elements.len() => parts,
          TypeKind::Error => vec![TypeId::ERROR; elements.len()],
          _ => {
            self.report_destructure_mismatch(pattern, ty, elements.len());
            vec![TypeId::ERROR; elements.len()]
          }
        };
        for (element, part) in elements.iter().zip(parts) {
          self.bind_irrefutable(element, part);
        }
      }
    }
  }
}

// Tra ve cho GAN dau tien vao `local`, chu khong phai chi co / khong.
// Truoc may ham nay tra ve bool, nhung 16.10 bo thu hep vi mot phep gan thi
// bao loi phai chi duoc vao dung phep gan do, khong thi doc xong van khong
// biet cai gi lam hong.
fn statements_assign(
  values: &HashMap<NodeId, ValueBinding>,
  statements: &[Stmt],
  local: LocalId,
) -> Option<Span> {
  statements
    .iter()
    .find_map(|statement| stmt_assigns(values, statement, local))
}

fn stmt_assigns(
  values: &HashMap<NodeId, ValueBinding>,
  statement: &Stmt,
  local: LocalId,
) -> Option<Span> {
  match &statement.kind {
    StmtKind::Assign(assign) => {
      if let ExprKind::Ident(_) = &assign.target.kind {
        if matches!(
          values.get(&assign.target.id),
          Some(ValueBinding::Local(id) | ValueBinding::Captured(id)) if *id == local
        ) {
          return Some(assign.target.span);
        }
      }
      expr_assigns(values, &assign.value, local)
    }
    StmtKind::Let(decl) => expr_assigns(values, &decl.value, local),
    StmtKind::Const(decl) => expr_assigns(values, &decl.value, local),
    StmtKind::Expr(expr) => expr_assigns(values, expr, local),
    StmtKind::If(statement) => expr_assigns(values, &statement.condition, local)
      .or_else(|| statements_assign(values, &statement.then_block.statements, local))
      .or_else(|| match &statement.else_branch {
        Some(ElseBranch::Block(block)) => {
          statements_assign(values, &block.statements, local)
        }
        Some(ElseBranch::If(nested)) => {
          let wrapped = Stmt {
            id: NodeId::NONE,
            kind: StmtKind::If(nested.as_ref().clone()),
            span: nested.span,
          };
          stmt_assigns(values, &wrapped, local)
        }
        None => None,
      }),
    StmtKind::While(statement) => expr_assigns(values, &statement.condition, local)
      .or_else(|| statements_assign(values, &statement.body.statements, local)),
    StmtKind::For(statement) => expr_assigns(values, &statement.iterable, local)
      .or_else(|| statements_assign(values, &statement.body.statements, local)),
    StmtKind::Match(statement) => {
      expr_assigns(values, &statement.scrutinee, local).or_else(|| {
        statement.arms.iter().find_map(|arm| match &arm.body {
          MatchArmBody::Block(block) => {
            statements_assign(values, &block.statements, local)
          }
          MatchArmBody::Stmt(inner) => stmt_assigns(values, inner, local),
        })
      })
    }
    StmtKind::Return(value) => value
      .as_ref()
      .and_then(|value| expr_assigns(values, value, local)),
    StmtKind::Fail(value) => expr_assigns(values, value, local),
    StmtKind::Break | StmtKind::Continue => None,
    StmtKind::Block(block) => statements_assign(values, &block.statements, local),
  }
}

fn expr_assigns(
  values: &HashMap<NodeId, ValueBinding>,
  expr: &Expr,
  local: LocalId,
) -> Option<Span> {
  match &expr.kind {
    ExprKind::Closure(closure) => statements_assign(values, &closure.body.statements, local),
    ExprKind::Group(inner)
    | ExprKind::Unary { operand: inner, .. }
    | ExprKind::NullPropagate(inner)
    | ExprKind::ErrorPropagate(inner)
    | ExprKind::Field { base: inner, .. }
    | ExprKind::TupleField { base: inner, .. }
    | ExprKind::TypeArgs { base: inner, .. } => expr_assigns(values, inner, local),
    ExprKind::Binary { lhs, rhs, .. } => {
      expr_assigns(values, lhs, local).or_else(|| expr_assigns(values, rhs, local))
    }
    ExprKind::Range { start, end, .. } => {
      expr_assigns(values, start, local).or_else(|| expr_assigns(values, end, local))
    }
    ExprKind::Index { base, index } => {
      expr_assigns(values, base, local).or_else(|| expr_assigns(values, index, local))
    }
    ExprKind::Call { callee, args } => expr_assigns(values, callee, local).or_else(|| {
      args.iter()
        .find_map(|argument| expr_assigns(values, &argument.value, local))
    }),
    ExprKind::Array(elements) | ExprKind::Set(elements) | ExprKind::Tuple(elements) => elements
      .iter()
      .find_map(|element| expr_assigns(values, element, local)),
    ExprKind::Map(entries) => entries.iter().find_map(|entry| {
      expr_assigns(values, &entry.key, local)
        .or_else(|| expr_assigns(values, &entry.value, local))
    }),
    ExprKind::StructLit(literal) => literal
      .fields
      .iter()
      .find_map(|field| expr_assigns(values, &field.value, local)),
    ExprKind::Str(literal) => literal.parts.iter().find_map(|part| match part {
      StringPart::Interp(inner) => expr_assigns(values, inner, local),
      StringPart::Text { .. } => None,
    }),
    ExprKind::Catch { operand, handler } => {
      expr_assigns(values, operand, local).or_else(|| match handler {
        CatchHandler::Discard(block) | CatchHandler::Bind { block, .. } => {
          statements_assign(values, &block.statements, local)
        }
        CatchHandler::Value(value) => expr_assigns(values, value, local),
      })
    }
    _ => None,
  }
}

fn contains_break(block: &Block) -> bool {
  block.statements.iter().any(statement_breaks)
}

fn statement_breaks(statement: &Stmt) -> bool {
  match &statement.kind {
    StmtKind::Break => true,
    StmtKind::Block(block) => contains_break(block),
    StmtKind::If(statement) => {
      contains_break(&statement.then_block)
        || match &statement.else_branch {
          Some(ElseBranch::Block(block)) => contains_break(block),
          Some(ElseBranch::If(nested)) => {
            let wrapped = Stmt {
              id: NodeId::NONE,
              kind: StmtKind::If(nested.as_ref().clone()),
              span: nested.span,
            };
            statement_breaks(&wrapped)
          }
          None => false,
        }
    }
    StmtKind::Match(statement) => statement.arms.iter().any(|arm| match &arm.body {
      MatchArmBody::Block(block) => contains_break(block),
      MatchArmBody::Stmt(inner) => statement_breaks(inner),
    }),
    _ => false,
  }
}

#[derive(Clone, Debug)]
enum Member {
  Field {
    owner: DefId,
    index: u32,
    ty: TypeId,
  },
  Length,
  Method {
    owner: DefId,
    func: FuncId,
    receiver: TypeId,
  },
  InterfaceMethod {
    interface: DefId,
    slot: u32,
    func: FuncId,
  },
  Builtin {
    method: BuiltinMethod,
    params: Vec<TypeId>,
    ret: TypeId,
  },
  Unknown,
}

impl Checker<'_> {
  fn check_value(&mut self, expr: &Expr, expected: Option<TypeId>) -> TypeId {
    let ty = self.check_expr(expr, expected);
    self.demand_value(ty, expr.span)
  }

  fn demand_value(&mut self, ty: TypeId, span: Span) -> TypeId {
    let resolved = self.context.shallow_resolve(ty);
    let TypeKind::Failable(inner) = *self.context.kind(resolved) else {
      return ty;
    };
    let shown = self.show(inner);
    self.report(
      CompileError::at(
        ErrorCode::UnhandledError,
        span,
        format!("this call can fail, so its `{shown}` is not available yet"),
      )
      .with_help("propagate it with `!`, or handle it with `catch`"),
    );
    inner
  }

  fn literal_expectation(&self, expected: Option<TypeId>) -> Option<TypeId> {
    let expected = expected?;
    let resolved = self.context.shallow_resolve(expected);
    match *self.context.kind(resolved) {
      TypeKind::Optional(inner) => Some(self.context.shallow_resolve(inner)),
      TypeKind::Var(_) => None,
      _ => Some(resolved),
    }
  }

  fn check_expr(&mut self, expr: &Expr, expected: Option<TypeId>) -> TypeId {
    let ty = self.check_expr_inner(expr, expected);
    self.record(expr.id, ty)
  }

  fn check_expr_inner(&mut self, expr: &Expr, expected: Option<TypeId>) -> TypeId {
    match &expr.kind {
      ExprKind::Int(magnitude) => {
        self.check_int_literal(*magnitude, false, expected, expr.span)
      }
      ExprKind::Float(_) => {
        match self
          .literal_expectation(expected)
          .map(|ty| self.context.kind(ty).clone())
        {
          Some(TypeKind::Int) | Some(TypeKind::Uint) => {
            self.report(
              CompileError::at(
                ErrorCode::LiteralOutOfRange,
                expr.span,
                "a float literal can never adopt an integer type",
              )
              .with_help("write an integer literal, or convert with `int(x)`"),
            );
          }
          _ => {}
        }
        TypeId::FLOAT
      }
      ExprKind::Char(_) => TypeId::CHAR,
      ExprKind::Bool(_) => TypeId::BOOL,
      ExprKind::Str(literal) => {
        for part in &literal.parts {
          if let StringPart::Interp(inner) = part {
            let ty = self.check_value(inner, None);
            self.check_interpolation(ty, inner.span);
          }
        }
        TypeId::STRING
      }
      ExprKind::Null => self.check_null(expected, expr.span),
      ExprKind::This => match self.signature.this {
        Some(ty) => ty,
        None => TypeId::ERROR,
      },
      ExprKind::Ident(name) => self.check_identifier(expr.id, name),
      ExprKind::Group(inner) => self.check_expr(inner, expected),
      ExprKind::Array(elements) => self.check_array(elements, expected, expr.span),
      ExprKind::Set(elements) => self.check_set(elements, expected, expr.span),
      ExprKind::Map(entries) => self.check_map(entries, expected, expr.span),
      ExprKind::Tuple(elements) => self.check_tuple(elements, expected),
      ExprKind::Closure(closure) => self.check_closure(closure, expected),
      ExprKind::Unary { op, operand } => self.check_unary(*op, operand, expected, expr.span),
      ExprKind::Binary { op, lhs, rhs } => {
        self.check_binary(*op, lhs, rhs, expected, expr.span)
      }
      ExprKind::Range {
        start,
        end,
        inclusive,
      } => {
        let _ = inclusive;
        let start_ty = self.check_value(start, Some(TypeId::INT));
        self.expect_assignable(TypeId::INT, start_ty, start.span, "a range endpoint");
        let end_ty = self.check_value(end, Some(TypeId::INT));
        self.expect_assignable(TypeId::INT, end_ty, end.span, "a range endpoint");
        self.prelude.range_type
      }
      ExprKind::Catch { operand, handler } => self.check_catch(operand, handler, expr.span),
      ExprKind::Field { base, name } => self.check_field_expr(expr, base, name, expected),
      ExprKind::TupleField {
        base,
        index,
        index_span,
      } => self.check_tuple_field(base, *index, *index_span),
      ExprKind::Index { base, index } => self.check_index(base, index, expr.span),
      ExprKind::Call { callee, args } => self.check_call(expr, callee, args, expected),
      ExprKind::NullPropagate(inner) => self.check_null_propagate(inner, expr.span),
      ExprKind::ErrorPropagate(inner) => self.check_error_propagate(inner, expr.span),
      ExprKind::TypeArgs { base, args } => {
        self.check_value(base, None);
        for arg in args {
          let _ = self.written_types.get(&arg.id);
        }
        self.report(
          CompileError::at(
            ErrorCode::TurbofishNotAllowed,
            expr.span,
            "explicit type arguments belong on a call or a struct literal",
          )
          .with_help("write `f::<int>(x)` or `Box::<int> { value: 1 }`"),
        );
        TypeId::ERROR
      }
      ExprKind::StructLit(_) => self.check_struct_literal(expr, expected),
    }
  }

  fn check_int_literal(
    &mut self,
    magnitude: u64,
    negated: bool,
    expected: Option<TypeId>,
    span: Span,
  ) -> TypeId {
    let target = self.literal_expectation(expected);
    let adopted = match target.map(|ty| self.context.kind(ty).clone()) {
      Some(TypeKind::Uint) => TypeId::UINT,
      Some(TypeKind::Float) => TypeId::FLOAT,
      _ => TypeId::INT,
    };
    match adopted {
      TypeId::INT => {
        let limit = if negated { 1u64 << 62 } else { i64::MAX as u64 };
        if magnitude > limit {
          self.report(
            CompileError::at(
              ErrorCode::LiteralOutOfRange,
              span,
              format!("`{magnitude}` does not fit in `int`"),
            )
            .with_note(
              "`int` is signed 64-bit; the largest value is 9223372036854775807",
            ),
          );
        }
      }
      TypeId::UINT if negated => {
        self.report(
          CompileError::at(
            ErrorCode::NegateUnsigned,
            span,
            "`uint` has no negative values",
          )
          .with_help("use `int` here, or drop the sign"),
        );
      }
      _ => {}
    }
    adopted
  }

  fn check_null(&mut self, expected: Option<TypeId>, span: Span) -> TypeId {
    let Some(expected) = expected else {
      let fresh = self.context.fresh_var();
      return self.context.optional_of(fresh);
    };
    let resolved = self.context.shallow_resolve(expected);
    match *self.context.kind(resolved) {
      TypeKind::Optional(_) | TypeKind::Error => resolved,
      TypeKind::Var(_) => {
        let fresh = self.context.fresh_var();
        let optional = self.context.optional_of(fresh);
        self.unify(resolved, optional);
        optional
      }
      _ => {
        let shown = self.show(resolved);
        self.report(
          CompileError::at(
            ErrorCode::TypeMismatch,
            span,
            format!("`null` is not a value of `{shown}`"),
          )
          .with_help(format!("declare the type as `{shown}?` to allow `null`")),
        );
        self.context.optional_of(resolved)
      }
    }
  }

  fn check_interpolation(&mut self, ty: TypeId, span: Span) {
    let resolved = self.context.shallow_resolve(ty);
    let primitive = matches!(
      self.context.kind(resolved),
      TypeKind::Bool
        | TypeKind::Int
        | TypeKind::Uint
        | TypeKind::Float
        | TypeKind::Char
        | TypeKind::String
        | TypeKind::Error
        | TypeKind::Never
    );
    if primitive {
      return;
    }
    if let TypeKind::Generic(generic) = *self.context.kind(resolved) {
      // tham so kieu co rang buoc thi stringable neu mot trong cac rang
      // buoc cho `to_string`. Monomorphisation the kieu cu the vao truoc
      // khi lower, nen cho nay khong can itable.
      if self.bound_supplies(generic, "to_string") {
        return;
      }
    }
    let stringable = self.prelude.stringable;
    if self.record_conformance(stringable, resolved).is_some() {
      return;
    }
    let shown = self.show(resolved);
    self.report(
      CompileError::at(
        ErrorCode::InvalidInterpolation,
        span,
        format!("`{shown}` cannot be written into a string"),
      )
      .with_note("interpolation takes a primitive, or a type with `fn to_string(): string`"),
    );
  }

  fn check_identifier(&mut self, node: NodeId, name: &Ident) -> TypeId {
    let Some(binding) = self.values.get(&node).copied() else {
      return TypeId::ERROR;
    };
    match binding {
      ValueBinding::Local(local) | ValueBinding::Captured(local) => self.local_type(local),
      ValueBinding::Field { owner, index } => self
        .context
        .def(owner)
        .as_struct()
        .and_then(|structure| structure.fields.get(index as usize))
        .map(|field| field.ty)
        .unwrap_or(TypeId::ERROR),
      ValueBinding::Method(_) => {
        self.report(
          CompileError::at(
            ErrorCode::UnknownIdentifier,
            name.span,
            format!("`{}` is a method and must be called", name.name),
          )
          .with_help(format!("write `{}()`", name.name)),
        );
        TypeId::ERROR
      }
      ValueBinding::Function(func) => self.function_value_type(func, name.span),
      ValueBinding::GlobalConst(global) => self.global_types[global.index()],
      ValueBinding::Module(_) => {
        self.report(
          CompileError::at(
            ErrorCode::UnknownIdentifier,
            name.span,
            format!("`{}` is a module, not a value", name.name),
          )
          .with_help(format!("reach into it: `{}.item`", name.name)),
        );
        TypeId::ERROR
      }
      ValueBinding::Type(def) => {
        let what = self.context.def(def).name.clone();
        self.report(
          CompileError::at(
            ErrorCode::UnknownIdentifier,
            name.span,
            format!("`{what}` is a type, not a value"),
          )
          .with_help(
            "name a variant with `Enum.Variant`, or build a value with `Type { ... }`",
          ),
        );
        TypeId::ERROR
      }
      ValueBinding::Predeclared(value) => {
        self.report(
          CompileError::at(
            ErrorCode::UnknownIdentifier,
            name.span,
            format!("`{}` is a builtin and must be called", value.spelling()),
          )
          .with_help(format!("write `{}(x)`", value.spelling())),
        );
        TypeId::ERROR
      }
      ValueBinding::Conversion(target) => {
        let shown = self.show(target);
        self.report(
          CompileError::at(
            ErrorCode::UnknownIdentifier,
            name.span,
            format!("`{shown}` is a type, not a value"),
          )
          .with_help(format!("a conversion is a call: `{shown}(x)`")),
        );
        TypeId::ERROR
      }
    }
  }

  fn function_value_type(&mut self, func: FuncId, span: Span) -> TypeId {
    let definition = self.context.func(func).clone();
    if definition.is_generic() {
      self.report(
        CompileError::at(
          ErrorCode::CannotInferType,
          span,
          format!(
            "`{}` is generic, so it has no single function type",
            definition.name
          ),
        )
        .with_help("call it instead, or wrap it in a closure"),
      );
      return TypeId::ERROR;
    }
    let variadic = definition
      .variadic_index()
      .map(|index| definition.params[index].ty);
    let params = definition
      .params
      .iter()
      .filter(|param| !param.variadic)
      .map(|param| param.ty)
      .collect();
    self.context.function(FnType {
      params,
      variadic,
      ret: definition.ret,
      failable: definition.failable,
    })
  }
}

impl Checker<'_> {
  fn check_array(&mut self, elements: &[Expr], expected: Option<TypeId>, span: Span) -> TypeId {
    let hint = match self
      .literal_expectation(expected)
      .map(|ty| self.context.kind(ty).clone())
    {
      Some(TypeKind::Array(element)) => Some(element),
      _ => None,
    };
    let mut element_type = hint;
    for element in elements {
      let ty = self.check_value(element, element_type);
      match element_type {
        Some(expected) => {
          self.expect_assignable(expected, ty, element.span, "this array element")
        }
        None => element_type = Some(ty),
      }
    }
    match element_type {
      Some(element) => self.context.array_of(element),
      None => {
        self.report_uninferable("array", span, "let items: [int] = []");
        TypeId::ERROR
      }
    }
  }

  fn check_set(&mut self, elements: &[Expr], expected: Option<TypeId>, span: Span) -> TypeId {
    let hint = match self
      .literal_expectation(expected)
      .map(|ty| self.context.kind(ty).clone())
    {
      Some(TypeKind::Set(element)) => Some(element),
      _ => None,
    };
    let mut element_type = hint;
    for element in elements {
      let ty = self.check_value(element, element_type);
      match element_type {
        Some(expected) => {
          self.expect_assignable(expected, ty, element.span, "this set element")
        }
        None => element_type = Some(ty),
      }
    }
    match element_type {
      Some(element) => {
        self.demand_hashable(element, span, "a set element");
        self.context.set_of(element)
      }
      None => {
        self.report_uninferable("set", span, "let ids: set<int> = set{}");
        TypeId::ERROR
      }
    }
  }

  fn check_map(
    &mut self,
    entries: &[crate::ast::MapEntry],
    expected: Option<TypeId>,
    span: Span,
  ) -> TypeId {
    // `{}` viet giong nhau cho ca hai loai, nen mot cai rong ma chu thich
    // `set<T>` thi la set rong chu khong phai map (spec 5)
    let expectation = self
      .literal_expectation(expected)
      .map(|ty| self.context.kind(ty).clone());
    if entries.is_empty() {
      if let Some(TypeKind::Set(element)) = expectation {
        self.demand_hashable(element, span, "a set element");
        return self.context.set_of(element);
      }
    }

    let hint = match expectation {
      Some(TypeKind::Map { key, value }) => Some((key, value)),
      _ => None,
    };
    let mut pair = hint;
    for entry in entries {
      let key = self.check_value(&entry.key, pair.map(|(key, _)| key));
      let value = self.check_value(&entry.value, pair.map(|(_, value)| value));
      match pair {
        Some((expected_key, expected_value)) => {
          self.expect_assignable(expected_key, key, entry.key.span, "this map key");
          self.expect_assignable(
            expected_value,
            value,
            entry.value.span,
            "this map value",
          );
        }
        None => pair = Some((key, value)),
      }
    }
    match pair {
      Some((key, value)) => {
        self.demand_hashable(key, span, "a map key");
        self.context.map_of(key, value)
      }
      None => {
        self.report_uninferable("map", span, "let users: [string: User] = {}");
        TypeId::ERROR
      }
    }
  }

  fn check_tuple(&mut self, elements: &[Expr], expected: Option<TypeId>) -> TypeId {
    let hint = match self
      .literal_expectation(expected)
      .map(|ty| self.context.kind(ty).clone())
    {
      Some(TypeKind::Tuple(parts)) if parts.len() == elements.len() => Some(parts),
      _ => None,
    };
    let mut parts = Vec::with_capacity(elements.len());
    for (index, element) in elements.iter().enumerate() {
      let expected = hint.as_ref().map(|parts| parts[index]);
      let ty = self.check_value(element, expected);
      if let Some(expected) = expected {
        self.expect_assignable(expected, ty, element.span, "this tuple element");
        parts.push(expected);
      } else {
        parts.push(ty);
      }
    }
    self.context.tuple_of(parts)
  }

  fn demand_hashable(&mut self, ty: TypeId, span: Span, position: &str) {
    let resolved = self.context.shallow_resolve(ty);
    if self.context.is_hashable(resolved) || resolved == TypeId::ERROR {
      return;
    }
    let shown = self.show(resolved);
    let code = if resolved == TypeId::FLOAT {
      ErrorCode::FloatNotHashable
    } else {
      ErrorCode::TypeMismatch
    };
    let mut error = CompileError::at(code, span, format!("`{shown}` cannot be {position}"));
    if resolved == TypeId::FLOAT {
      error = error.with_note("`float` has no total order and `NaN` is not equal to itself");
    }
    self.report(error);
  }

  fn report_uninferable(&mut self, what: &str, span: Span, example: &str) {
    self.report(
      CompileError::at(
        ErrorCode::CannotInferType,
        span,
        format!("an empty {what} literal has nothing to infer its type from"),
      )
      .with_help(format!("annotate the binding: `{example}`")),
    );
  }

  fn check_closure(&mut self, closure: &ClosureExpr, expected: Option<TypeId>) -> TypeId {
    let hint = match self
      .literal_expectation(expected)
      .map(|ty| self.context.kind(ty).clone())
    {
      Some(TypeKind::Function(signature)) => Some(signature),
      _ => None,
    };

    let mut params = Vec::with_capacity(closure.params.len());
    let mut variadic = None;
    for param in &closure.params {
      let ty = self
        .written_types
        .get(&param.ty.id)
        .copied()
        .unwrap_or(TypeId::ERROR);
      if matches!(param.kind, crate::ast::ParamKind::Variadic) {
        variadic = Some(ty);
        let array = self.context.array_of(ty);
        self.bind_local(&param.name, array);
      } else {
        params.push(ty);
        self.bind_local(&param.name, ty);
      }
    }

    let (ret, failable) = match &closure.return_type {
      Some(written) => {
        let ty = self
          .written_types
          .get(&written.id)
          .copied()
          .unwrap_or(TypeId::ERROR);
        match *self.context.kind(ty) {
          TypeKind::Failable(inner) => (inner, true),
          _ => (ty, false),
        }
      }
      None => (TypeId::VOID, false),
    };

    let this = self.signature.this;
    let owner = self.signature.owner;
    let signature = std::mem::replace(
      &mut self.signature,
      Signature {
        ret,
        failable,
        this,
        owner,
      },
    );
    // closure co the chay sau khi vung thu hep da het, nen khong co thu
    // hep nao song sot vao trong than no (D-25)
    let narrowings = std::mem::take(&mut self.narrowings);
    self.closure_depth += 1;

    let diverges = self.check_block(&closure.body);
    if ret != TypeId::VOID && !diverges {
      let shown = self.show(ret);
      self.report(CompileError::at(
        ErrorCode::MissingReturn,
        closure.body.span,
        format!("this closure must return `{shown}` on every path"),
      ));
    }

    self.closure_depth -= 1;
    self.narrowings = narrowings;
    self.signature = signature;

    let built = self.context.function(FnType {
      params,
      variadic,
      ret,
      failable,
    });
    if let Some(hint) = hint {
      let wanted = self.context.function(hint);
      self.unify(wanted, built);
    }
    built
  }

  fn check_unary(
    &mut self,
    op: UnaryOp,
    operand: &Expr,
    expected: Option<TypeId>,
    span: Span,
  ) -> TypeId {
    match op {
      UnaryOp::Not => {
        let ty = self.check_value(operand, Some(TypeId::BOOL));
        if !self.is_boolish(ty) {
          let shown = self.show(ty);
          self.report(
            CompileError::at(
              ErrorCode::LogicalOnNonBool,
              span,
              format!("`!` needs a `bool`, but this is `{shown}`"),
            )
            .with_help(self.suggest_boolean_test(ty)),
          );
        }
        TypeId::BOOL
      }
      UnaryOp::Neg => {
        // `-9223372036854775808` la mot literal, khong phai dau tru
        // dat truoc mot so vuot khoang (3.5.3)
        if let ExprKind::Int(magnitude) = operand.kind {
          let ty = self.check_int_literal(magnitude, true, expected, operand.span);
          self.record(operand.id, ty);
          return ty;
        }
        let ty = self.check_value(operand, expected);
        let resolved = self.context.shallow_resolve(ty);
        match self.context.kind(resolved) {
          TypeKind::Int | TypeKind::Float | TypeKind::Error | TypeKind::Never => resolved,
          TypeKind::Uint => {
            self.report(
              CompileError::at(
                ErrorCode::NegateUnsigned,
                span,
                "`uint` is unsigned, so it cannot be negated",
              )
              .with_help("convert first: `-int(x)`"),
            );
            resolved
          }
          TypeKind::Char => {
            self.report(
              CompileError::at(
                ErrorCode::CharArithmetic,
                span,
                "`char` has no arithmetic",
              )
              .with_help("convert to a number first: `uint(c)`"),
            );
            TypeId::ERROR
          }
          _ => {
            let shown = self.show(resolved);
            self.report(CompileError::at(
              ErrorCode::ArithmeticOnNonNumeric,
              span,
              format!("`-` needs `int` or `float`, but this is `{shown}`"),
            ));
            TypeId::ERROR
          }
        }
      }
    }
  }

  fn is_boolish(&self, ty: TypeId) -> bool {
    matches!(
      self.context.kind(self.context.shallow_resolve(ty)),
      TypeKind::Bool | TypeKind::Error | TypeKind::Never
    )
  }

  fn suggest_boolean_test(&self, ty: TypeId) -> String {
    match self.context.kind(self.context.shallow_resolve(ty)) {
      TypeKind::Int | TypeKind::Uint | TypeKind::Float => "compare it: `x != 0`".to_string(),
      TypeKind::String => "compare it: `x != \"\"`".to_string(),
      TypeKind::Optional(_) => "test it: `x != null`".to_string(),
      _ => "Pump has no truthiness; write an explicit comparison".to_string(),
    }
  }

  fn check_binary(
    &mut self,
    op: BinaryOp,
    lhs: &Expr,
    rhs: &Expr,
    expected: Option<TypeId>,
    span: Span,
  ) -> TypeId {
    use BinaryOp::*;
    if op.is_logical() {
      let left = self.check_value(lhs, Some(TypeId::BOOL));
      let right = self.check_value(rhs, Some(TypeId::BOOL));
      for (ty, operand) in [(left, lhs), (right, rhs)] {
        if !self.is_boolish(ty) {
          let shown = self.show(ty);
          self.report(
            CompileError::at(
              ErrorCode::LogicalOnNonBool,
              operand.span,
              format!(
                "`{}` needs `bool` operands, but this is `{shown}`",
                op.spelling()
              ),
            )
            .with_help(self.suggest_boolean_test(ty)),
          );
        }
      }
      return TypeId::BOOL;
    }

    // thu tu toan hang chi quan trong o cho: literal chua co kieu thi lay
    // kieu tu ben kia chu khong roi ve `int`
    let numeric_hint = expected.filter(|_| op.is_arithmetic() || op.is_bitwise());
    let (left, right) = if is_untyped_literal(lhs) && !is_untyped_literal(rhs) {
      let right = self.check_value(rhs, numeric_hint);
      let left = self.check_value(lhs, Some(right));
      (left, right)
    } else {
      let left = self.check_value(lhs, numeric_hint);
      let right = self.check_value(rhs, Some(left));
      (left, right)
    };

    match op {
      Eq | Ne => self.check_equality(op, left, right, lhs, rhs, span),
      Lt | Gt | Le | Ge => self.check_ordering(op, left, right, span),
      Shl | Shr => self.check_shift(op, left, right, lhs, rhs, span),
      BitAnd | BitXor | BitOr => self.check_bitwise(op, left, right, span),
      Add | Sub | Mul | Div | Rem => self.check_arithmetic(op, left, right, span),
      And | Or => TypeId::BOOL,
    }
  }

  fn check_equality(
    &mut self,
    op: BinaryOp,
    left: TypeId,
    right: TypeId,
    lhs: &Expr,
    rhs: &Expr,
    span: Span,
  ) -> TypeId {
    // `x == null` la phep so sanh duy nhat bang qua ranh gioi optional
    let comparing_null =
      matches!(lhs.kind, ExprKind::Null) || matches!(rhs.kind, ExprKind::Null);
    if comparing_null {
      let other = if matches!(lhs.kind, ExprKind::Null) {
        right
      } else {
        left
      };
      let resolved = self.context.shallow_resolve(other);
      if !matches!(
        self.context.kind(resolved),
        TypeKind::Optional(_) | TypeKind::Error | TypeKind::Never
      ) {
        let shown = self.show(resolved);
        self.report(
          CompileError::at(
            ErrorCode::ComparisonNotSupported,
            span,
            format!("`{shown}` is never `null`"),
          )
          .with_help(format!("declare it as `{shown}?` if it can be absent")),
        );
      }
      return TypeId::BOOL;
    }

    if !self.assignable(left, right) && !self.assignable(right, left) {
      let a = self.show(left);
      let b = self.show(right);
      self.report(
        CompileError::at(
          ErrorCode::ComparisonNotSupported,
          span,
          format!("cannot compare `{a}` with `{b}`"),
        )
        .with_note("`==` compares two values of the same type"),
      );
      return TypeId::BOOL;
    }
    let _ = op;
    TypeId::BOOL
  }

  fn check_ordering(&mut self, op: BinaryOp, left: TypeId, right: TypeId, span: Span) -> TypeId {
    if !self.assignable(left, right) && !self.assignable(right, left) {
      let a = self.show(left);
      let b = self.show(right);
      self.report(CompileError::at(
        ErrorCode::ComparisonNotSupported,
        span,
        format!("cannot order `{a}` against `{b}`"),
      ));
      return TypeId::BOOL;
    }
    let resolved = self.context.shallow_resolve(left);
    let orderable = matches!(
      self.context.kind(resolved),
      TypeKind::Int
        | TypeKind::Uint
        | TypeKind::Float
        | TypeKind::Char
        | TypeKind::String
        | TypeKind::Error
        | TypeKind::Never
    );
    if !orderable {
      let shown = self.show(resolved);
      self.report(
        CompileError::at(
          ErrorCode::ComparisonNotSupported,
          span,
          format!("`{}` is not defined on `{shown}`", op.spelling()),
        )
        .with_note("ordering is defined on the numbers, `char` and `string`"),
      );
    }
    TypeId::BOOL
  }

  fn check_shift(
    &mut self,
    op: BinaryOp,
    left: TypeId,
    right: TypeId,
    lhs: &Expr,
    rhs: &Expr,
    span: Span,
  ) -> TypeId {
    let _ = span;
    for (ty, operand) in [(left, lhs), (right, rhs)] {
      if !self.context.is_integer(ty) && ty != TypeId::ERROR {
        let shown = self.show(ty);
        self.report(CompileError::at(
          ErrorCode::BitwiseOnNonInteger,
          operand.span,
          format!(
            "`{}` needs `int` or `uint`, but this is `{shown}`",
            op.spelling()
          ),
        ));
        return TypeId::ERROR;
      }
    }
    left
  }

  fn check_bitwise(&mut self, op: BinaryOp, left: TypeId, right: TypeId, span: Span) -> TypeId {
    if !self.context.is_integer(left) || !self.context.is_integer(right) {
      let a = self.show(left);
      let b = self.show(right);
      if left != TypeId::ERROR && right != TypeId::ERROR {
        self.report(CompileError::at(
          ErrorCode::BitwiseOnNonInteger,
          span,
          format!(
            "`{}` needs `int` or `uint`, but these are `{a}` and `{b}`",
            op.spelling()
          ),
        ));
      }
      return TypeId::ERROR;
    }
    if !self.unify(left, right) {
      let a = self.show(left);
      let b = self.show(right);
      self.report(
        CompileError::at(
          ErrorCode::NoImplicitConversion,
          span,
          format!("`{}` needs both operands to be the same type, but these are `{a}` and `{b}`", op.spelling()),
        )
        .with_help("convert one of them explicitly"),
      );
      return TypeId::ERROR;
    }
    left
  }

  fn check_arithmetic(
    &mut self,
    op: BinaryOp,
    left: TypeId,
    right: TypeId,
    span: Span,
  ) -> TypeId {
    let resolved = self.context.shallow_resolve(left);
    if matches!(self.context.kind(resolved), TypeKind::String) {
      if op != BinaryOp::Add {
        self.report(
          CompileError::at(
            ErrorCode::ArithmeticOnNonNumeric,
            span,
            format!("`{}` is not defined on `string`", op.spelling()),
          )
          .with_note("only `+` concatenates"),
        );
        return TypeId::STRING;
      }
      self.expect_assignable(TypeId::STRING, right, span, "the right-hand side of `+`");
      return TypeId::STRING;
    }
    if matches!(self.context.kind(resolved), TypeKind::Char) {
      self.report(
        CompileError::at(ErrorCode::CharArithmetic, span, "`char` has no arithmetic")
          .with_help("convert to a number first: `uint(c)`"),
      );
      return TypeId::ERROR;
    }
    if !self.context.is_numeric(resolved) {
      if resolved != TypeId::ERROR && !matches!(self.context.kind(resolved), TypeKind::Never)
      {
        let shown = self.show(resolved);
        self.report(CompileError::at(
          ErrorCode::ArithmeticOnNonNumeric,
          span,
          format!(
            "`{}` needs a numeric type, but this is `{shown}`",
            op.spelling()
          ),
        ));
      }
      return TypeId::ERROR;
    }
    if !self.unify(left, right) {
      let a = self.show(left);
      let b = self.show(right);
      self.report(
        CompileError::at(
          ErrorCode::NoImplicitConversion,
          span,
          format!("cannot apply `{}` to `{a}` and `{b}`", op.spelling()),
        )
        .with_note("Pump has no implicit numeric conversions, `int` to `float` included")
        .with_help("convert one side: `float(n)`"),
      );
      return TypeId::ERROR;
    }
    resolved
  }
}

fn is_untyped_literal(expr: &Expr) -> bool {
  match &expr.kind {
    ExprKind::Int(_) | ExprKind::Float(_) => true,
    ExprKind::Group(inner) => is_untyped_literal(inner),
    ExprKind::Unary {
      op: UnaryOp::Neg,
      operand,
    } => is_untyped_literal(operand),
    _ => false,
  }
}

impl Checker<'_> {
  fn check_assignment(&mut self, assign: &AssignStmt) {
    let target = self.check_assign_target(&assign.target);
    let value = self.check_value(&assign.value, target);

    if let Some(target) = target {
      match assign.op.binary_op() {
        None => self.expect_assignable(target, value, assign.value.span, "this assignment"),
        Some(op) => {
          let produced = self.check_arithmetic(op, target, value, assign.span);
          self.expect_assignable(
            target,
            produced,
            assign.span,
            "this compound assignment",
          );
        }
      }
    }

    // ghi vao mot local da thu hep thi ket thuc thu hep ngay tai day. D-25
    // con pha no tren ca vung nao chua phep ghi do nua, cai do `surviving`
    // lo khi buoc vao vung.
    if let Some(local) = self.assigned_local(&assign.target) {
      self.narrowings.remove(&local);
    }
  }

  fn check_assign_target(&mut self, target: &Expr) -> Option<TypeId> {
    match &target.kind {
      ExprKind::Ident(name) => self.check_assign_identifier(target, name),
      ExprKind::This => {
        self.report(
          CompileError::at(
            ErrorCode::CannotAssignToThis,
            target.span,
            "`this` is the receiver itself and cannot be reassigned",
          )
          .with_help("assign to one of its fields: `this.f = x`"),
        );
        None
      }
      ExprKind::Field { .. } => {
        let ty = self.check_expr(target, None);
        match self.field_accesses.get(&target.id).copied() {
          Some(FieldAccess::Field { .. }) => Some(ty),
          Some(FieldAccess::Length) => {
            self.report(
              CompileError::at(
                ErrorCode::InvalidAssignmentTarget,
                target.span,
                "`length` is computed, not stored",
              )
              .with_help("change the collection instead: `push`, `pop`, `remove`"),
            );
            None
          }
          None => None,
        }
      }
      ExprKind::Index { .. } => {
        let ty = self.check_expr(target, None);
        (ty != TypeId::ERROR).then_some(ty)
      }
      _ => {
        self.report(
          CompileError::at(
            ErrorCode::InvalidAssignmentTarget,
            target.span,
            "only a name, a field or an index may be assigned to",
          )
          .with_note("assignment is a statement in Pump, never an expression"),
        );
        None
      }
    }
  }

  fn check_assign_identifier(&mut self, target: &Expr, name: &Ident) -> Option<TypeId> {
    let binding = self.values.get(&target.id).copied()?;
    match binding {
      ValueBinding::Local(local) | ValueBinding::Captured(local) => {
        let entry = self.locals[local.index()].clone();
        // lay kieu khai bao chu khong bao gio lay kieu da thu hep:
        // `user = null` van hop le voi `User?` ke ca khi dang o trong
        // `if user != null`
        let ty = self.local_types[local.index()];
        self.record(target.id, ty);
        if entry.reassignable {
          return Some(ty);
        }
        let captured_loop_binding = matches!(binding, ValueBinding::Captured(_))
          && entry.origin == LocalOrigin::LoopBinding;
        let error = if captured_loop_binding {
          CompileError::at(
            ErrorCode::MutableCaptureOfLoopBinding,
            name.span,
            format!(
              "`{}` is a `for` binding captured by this closure",
              name.name
            ),
          )
          .with_secondary(entry.span, "bound fresh on every iteration")
        } else {
          match entry.origin {
            LocalOrigin::LoopBinding => CompileError::at(
              ErrorCode::CannotAssignToLoopBinding,
              name.span,
              format!("`{}` is bound fresh on every iteration", name.name),
            )
            .with_secondary(entry.span, "the `for` binding is here")
            .with_help("copy it into a `let` first"),
            LocalOrigin::CatchBinding => CompileError::at(
              ErrorCode::CannotAssignToConst,
              name.span,
              format!("`{}` is the error bound by this `catch`", name.name),
            )
            .with_secondary(entry.span, "bound here"),
            LocalOrigin::PatternBinding => CompileError::at(
              ErrorCode::CannotAssignToConst,
              name.span,
              format!("`{}` is bound by a pattern and is immutable", name.name),
            )
            .with_secondary(entry.span, "bound here"),
            _ => CompileError::at(
              ErrorCode::CannotAssignToConst,
              name.span,
              format!("`{}` is a `const` binding", name.name),
            )
            .with_secondary(entry.span, "declared `const` here")
            .with_help("declare it with `let` to allow reassignment"),
          }
        };
        self.report(error);
        Some(ty)
      }
      ValueBinding::Field { owner, index } => {
        let ty = self
          .context
          .def(owner)
          .as_struct()
          .and_then(|structure| structure.fields.get(index as usize))
          .map(|field| field.ty)
          .unwrap_or(TypeId::ERROR);
        self.record(target.id, ty);
        self.field_accesses
          .insert(target.id, FieldAccess::Field { owner, index });
        Some(ty)
      }
      ValueBinding::GlobalConst(global) => {
        let ty = self.global_types[global.index()];
        let declared = self.globals[global.index()].span;
        self.record(target.id, ty);
        self.report(
          CompileError::at(
            ErrorCode::CannotAssignToConst,
            name.span,
            format!("`{}` is a module constant", name.name),
          )
          .with_secondary(declared, "declared here")
          .with_note("module level admits `const` only; Pump 1.0 has no mutable globals"),
        );
        Some(ty)
      }
      _ => {
        self.report(CompileError::at(
          ErrorCode::InvalidAssignmentTarget,
          name.span,
          format!("`{}` does not name a storage location", name.name),
        ));
        None
      }
    }
  }

  fn assigned_local(&self, target: &Expr) -> Option<LocalId> {
    if !matches!(target.kind, ExprKind::Ident(_)) {
      return None;
    }
    match self.values.get(&target.id)? {
      ValueBinding::Local(local) | ValueBinding::Captured(local) => Some(*local),
      _ => None,
    }
  }
}

impl Checker<'_> {
  fn check_condition(&mut self, expr: &Expr) -> Facts {
    match &expr.kind {
      ExprKind::Group(inner) => {
        let facts = self.check_condition(inner);
        self.record(expr.id, TypeId::BOOL);
        facts
      }
      ExprKind::Unary {
        op: UnaryOp::Not,
        operand,
      } => {
        let facts = self.check_condition(operand);
        self.record(expr.id, TypeId::BOOL);
        facts.inverted()
      }
      ExprKind::Binary {
        op: BinaryOp::And,
        lhs,
        rhs,
      } => {
        let left = self.check_condition(lhs);
        let saved = self.narrowings.clone();
        for (&local, &ty) in &left.when_true {
          self.narrowings.insert(local, ty);
        }
        let right = self.check_condition(rhs);
        self.narrowings = saved;

        let mut when_true = left.when_true;
        when_true.extend(right.when_true);
        self.record(expr.id, TypeId::BOOL);
        Facts {
          when_true,
          when_false: Narrowings::new(),
        }
      }
      ExprKind::Binary {
        op: BinaryOp::Or,
        lhs,
        rhs,
      } => {
        self.check_condition(lhs);
        self.check_condition(rhs);
        self.record(expr.id, TypeId::BOOL);
        // `||` co y khong chung minh duoc gi o ca hai nhanh (D-25)
        Facts::default()
      }
      ExprKind::Binary {
        op: op @ (BinaryOp::Eq | BinaryOp::Ne),
        lhs,
        rhs,
      } => {
        let ty = self.check_expr(expr, Some(TypeId::BOOL));
        self.demand_bool(ty, expr.span);
        self.null_test_facts(*op, lhs, rhs)
      }
      _ => {
        let ty = self.check_value(expr, Some(TypeId::BOOL));
        self.demand_bool(ty, expr.span);
        Facts::default()
      }
    }
  }

  fn demand_bool(&mut self, ty: TypeId, span: Span) {
    if self.is_boolish(ty) {
      return;
    }
    let shown = self.show(ty);
    self.report(
      CompileError::at(
        ErrorCode::NoTruthiness,
        span,
        format!("a condition must be `bool`, but this is `{shown}`"),
      )
      .with_caret(format!("this is `{shown}`"))
      .with_help(self.suggest_boolean_test(ty)),
    );
  }

  fn null_test_facts(&mut self, op: BinaryOp, lhs: &Expr, rhs: &Expr) -> Facts {
    let subject = if matches!(peel_groups(rhs).kind, ExprKind::Null) {
      lhs
    } else if matches!(peel_groups(lhs).kind, ExprKind::Null) {
      rhs
    } else {
      return Facts::default();
    };
    let Some((local, inner)) = self.narrowable_local(subject) else {
      return Facts::default();
    };
    let mut facts = Facts::default();
    match op {
      BinaryOp::Ne => {
        facts.when_true.insert(local, inner);
      }
      BinaryOp::Eq => {
        facts.when_false.insert(local, inner);
      }
      _ => {}
    }
    facts
  }

  /// The assignment that ended `expr`'s narrowing, when there was one.
  fn defeated_at(&self, expr: &Expr) -> Option<Span> {
    let expr = peel_groups(expr);
    if !matches!(expr.kind, ExprKind::Ident(_)) {
      return None;
    }
    let local = match self.values.get(&expr.id)? {
      ValueBinding::Local(local) | ValueBinding::Captured(local) => *local,
      _ => return None,
    };
    self.defeated.get(&local).copied()
  }

  fn narrowable_local(&self, expr: &Expr) -> Option<(LocalId, TypeId)> {
    let expr = peel_groups(expr);
    if !matches!(expr.kind, ExprKind::Ident(_)) {
      return None;
    }
    let local = match self.values.get(&expr.id)? {
      ValueBinding::Local(local) | ValueBinding::Captured(local) => *local,
      _ => return None,
    };
    let ty = self.context.shallow_resolve(self.local_type(local));
    match *self.context.kind(ty) {
      TypeKind::Optional(inner) => Some((local, inner)),
      _ => None,
    }
  }
}

fn peel_groups(expr: &Expr) -> &Expr {
  let mut current = expr;
  while let ExprKind::Group(inner) = &current.kind {
    current = inner;
  }
  current
}

const WITNESS_LIMIT: usize = 4;

const EXPANSION_LIMIT: usize = 64;

impl Checker<'_> {
  fn check_match(&mut self, statement: &MatchStmt) -> bool {
    let scrutinee = self.check_value(&statement.scrutinee, None);
    let scrutinee = self.context.shallow_resolve(scrutinee);

    let mut matrix: Vec<Vec<Deconstructed>> = Vec::new();
    let mut every_arm_diverges = !statement.arms.is_empty();

    for arm in &statement.arms {
      let saved = self.narrowings.clone();
      self.check_pattern(&arm.pattern, scrutinee);

      let rows = self.deconstruct(&arm.pattern, scrutinee);
      let guarded = arm.guard.is_some();
      if !guarded {
        let reachable = rows
          .iter()
          .any(|row| self.is_useful(&matrix, std::slice::from_ref(row)));
        if !reachable {
          self.report(
            CompileError::at(
              ErrorCode::UnreachableMatchArm,
              arm.pattern.span,
              "an earlier arm already covers every value this one matches",
            )
            .with_note("arms are tried in source order"),
          );
        }
        matrix.extend(rows.into_iter().map(|row| vec![row]));
      }

      if let Some(guard) = &arm.guard {
        let ty = self.check_value(guard, Some(TypeId::BOOL));
        self.demand_bool(ty, guard.span);
      }

      let diverges = match &arm.body {
        MatchArmBody::Block(block) => self.check_block(block),
        MatchArmBody::Stmt(inner) => self.check_stmt(inner, &[]),
      };
      every_arm_diverges &= diverges;
      self.narrowings = saved;
    }

    let query = vec![Deconstructed::wildcard(scrutinee)];
    let missing = self.witnesses(&matrix, &query);
    if !missing.is_empty() {
      let shown = self.show(scrutinee);
      let mut error = CompileError::at(
        ErrorCode::NonExhaustiveMatch,
        statement.span,
        format!("this `match` on `{shown}` does not cover every value"),
      )
      .with_caret("some values reach no arm");
      for witness in missing.iter().take(WITNESS_LIMIT) {
        let rendered = self.render_witness(&witness[0]);
        error = error.with_note(format!("`{rendered}` is not covered"));
      }
      if missing.len() > WITNESS_LIMIT {
        error = error.with_note(format!("and {} more", missing.len() - WITNESS_LIMIT));
      }
      error = error.with_help("add the missing arms, or a `_` arm");
      self.report(error);
      return false;
    }

    every_arm_diverges
  }

  fn check_pattern(&mut self, pattern: &Pattern, expected: TypeId) {
    let expected = self.context.shallow_resolve(expected);
    self.pattern_types.insert(pattern.id, expected);

    // moi pattern tru `_`, mot binding va `null` deu nhin xuyen qua `T?`,
    // co the thi `match opt { null => ..., 1 => ... }` moi check duoc
    // (13.4.4)
    let target = match *self.context.kind(expected) {
      TypeKind::Optional(inner) if !covers_the_whole_optional(&pattern.kind) => inner,
      _ => expected,
    };

    match &pattern.kind {
      PatternKind::Wildcard => {}
      PatternKind::Binding(name) => {
        self.bind_local(name, expected);
      }
      PatternKind::Null => {
        if !matches!(
          self.context.kind(expected),
          TypeKind::Optional(_) | TypeKind::Error | TypeKind::Never
        ) {
          let shown = self.show(expected);
          self.report(
            CompileError::at(
              ErrorCode::PatternTypeMismatch,
              pattern.span,
              format!("`{shown}` is never `null`"),
            )
            .with_help(format!("declare it as `{shown}?` if it can be absent")),
          );
        }
      }
      PatternKind::Bool(_) => self.expect_pattern_type(TypeId::BOOL, target, pattern.span),
      PatternKind::Int {
        magnitude,
        negative,
      } => self.check_int_pattern(*magnitude, *negative, target, pattern.span),
      PatternKind::Char(_) => self.expect_pattern_type(TypeId::CHAR, target, pattern.span),
      PatternKind::Str(_) => self.expect_pattern_type(TypeId::STRING, target, pattern.span),
      PatternKind::Range {
        start,
        end,
        inclusive,
      } => self.check_range_pattern(*start, *end, *inclusive, target, pattern.span),
      PatternKind::Variant {
        enum_name,
        variant,
        payload,
      } => {
        self.check_variant_pattern(pattern, enum_name, variant, payload.as_deref(), target)
      }
      PatternKind::Struct { fields, rest, .. } => {
        self.check_struct_pattern(pattern, fields, *rest, target)
      }
      PatternKind::Tuple(elements) => self.check_tuple_pattern(pattern, elements, target),
      PatternKind::Or(alternatives) => self.check_or_pattern(alternatives, expected),
    }
  }

  fn expect_pattern_type(&mut self, wanted: TypeId, actual: TypeId, span: Span) {
    if actual == TypeId::ERROR || self.assignable(actual, wanted) {
      return;
    }
    let expected = self.show(wanted);
    let found = self.show(actual);
    self.report(
      CompileError::at(
        ErrorCode::PatternTypeMismatch,
        span,
        format!("this pattern matches `{expected}`, but the value is `{found}`"),
      )
      .with_caret(format!("a `{expected}` pattern")),
    );
  }

  fn check_int_pattern(&mut self, magnitude: u64, negative: bool, target: TypeId, span: Span) {
    if target == TypeId::ERROR {
      return;
    }
    if !self.context.is_integer(target) {
      self.expect_pattern_type(TypeId::INT, target, span);
      return;
    }
    if negative && matches!(self.context.kind(target), TypeKind::Uint) {
      self.report(
        CompileError::at(
          ErrorCode::NegateUnsigned,
          span,
          "`uint` has no negative values, so this pattern can never match",
        )
        .with_help("drop the sign, or match on an `int`"),
      );
      return;
    }
    let limit = if negative {
      1u64 << 63
    } else {
      i64::MAX as u64
    };
    if matches!(self.context.kind(target), TypeKind::Int) && magnitude > limit {
      self.report(CompileError::at(
        ErrorCode::LiteralOutOfRange,
        span,
        format!("`{magnitude}` does not fit in `int`, so this pattern can never match"),
      ));
    }
  }

  fn check_range_pattern(
    &mut self,
    start: RangeEndpoint,
    end: RangeEndpoint,
    inclusive: bool,
    target: TypeId,
    span: Span,
  ) {
    let ordered = match (start, end) {
      (RangeEndpoint::Int { .. }, RangeEndpoint::Int { .. }) => {
        if !self.context.is_integer(target) && target != TypeId::ERROR {
          self.expect_pattern_type(TypeId::INT, target, span);
          return;
        }
        endpoint_value(start) <= endpoint_value(end)
      }
      (RangeEndpoint::Char(_), RangeEndpoint::Char(_)) => {
        self.expect_pattern_type(TypeId::CHAR, target, span);
        endpoint_value(start) <= endpoint_value(end)
      }
      _ => {
        self.report(
          CompileError::at(
            ErrorCode::InvalidRangePattern,
            span,
            "both endpoints must be integers, or both must be characters",
          )
          .with_note("a range pattern never mixes the two"),
        );
        return;
      }
    };
    if !ordered {
      self.report(
        CompileError::at(
          ErrorCode::InvalidRangePattern,
          span,
          "the low endpoint is above the high one, so this range is empty",
        )
        .with_help("swap the endpoints"),
      );
      return;
    }
    if !inclusive && endpoint_value(start) == endpoint_value(end) {
      self.report(
        CompileError::at(
          ErrorCode::InvalidRangePattern,
          span,
          "an exclusive range with equal endpoints matches nothing",
        )
        .with_help("write `..=` to include the endpoint"),
      );
    }
  }

  fn check_variant_pattern(
    &mut self,
    pattern: &Pattern,
    enum_name: &Ident,
    variant: &Ident,
    payload: Option<&[Pattern]>,
    target: TypeId,
  ) {
    let Some(declared) = self.pattern_defs.get(&pattern.id).copied() else {
      // resolver khong tim thay kieu va da bao roi
      return;
    };
    let Some((def, args)) = self.named_pattern_subject(target, pattern.span, enum_name) else {
      return;
    };
    if def != declared {
      self.report_pattern_subject_mismatch(declared, target, pattern.span);
      return;
    }
    let Some(enumeration) = self.context.def(def).as_enum().cloned() else {
      let shown = self.context.def(def).name.clone();
      self.report(
        CompileError::at(
          ErrorCode::NotAnEnum,
          pattern.span,
          format!("`{shown}` is not an enum, so it has no variants"),
        )
        .with_help("match a struct with `Name { .. }`"),
      );
      return;
    };
    let Some(index) = enumeration.variant_index(&variant.name) else {
      let owner = self.context.def(def).name.clone();
      let names: Vec<&str> = enumeration
        .variants
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
      self.report(
        CompileError::at(
          ErrorCode::UnknownVariant,
          variant.span,
          format!("`{owner}` has no variant `{}`", variant.name),
        )
        .with_note(format!("its variants are {}", join_names(&names))),
      );
      return;
    };

    let payload_types: Vec<TypeId> = enumeration.variants[index]
      .payload
      .clone()
      .into_iter()
      .map(|ty| self.context.substitute(ty, GenericOwner::Type(def), &args))
      .collect();

    match payload {
      None => {
        if !payload_types.is_empty() {
          let owner = self.context.def(def).name.clone();
          let holes = vec!["_"; payload_types.len()].join(", ");
          self.report(
            CompileError::at(
              ErrorCode::PatternTypeMismatch,
              pattern.span,
              format!(
                "`{}` carries {} value{}, so it needs a payload pattern",
                variant.name,
                payload_types.len(),
                if payload_types.len() == 1 { "" } else { "s" }
              ),
            )
            .with_help(format!("write `{owner}.{}({holes})`", variant.name)),
          );
        }
      }
      Some(elements) => {
        if elements.len() != payload_types.len() {
          self.report(CompileError::at(
            ErrorCode::WrongArgumentCount,
            pattern.span,
            format!(
              "`{}` carries {} value{}, but this pattern binds {}",
              variant.name,
              payload_types.len(),
              if payload_types.len() == 1 { "" } else { "s" },
              elements.len()
            ),
          ));
        }
        for (element, ty) in elements.iter().zip(payload_types) {
          self.check_pattern(element, ty);
        }
        for element in elements.iter().skip(
          enumeration.variants[index]
            .payload
            .len()
            .min(elements.len()),
        ) {
          self.check_pattern(element, TypeId::ERROR);
        }
      }
    }
  }

  fn check_struct_pattern(
    &mut self,
    pattern: &Pattern,
    fields: &[crate::ast::FieldPattern],
    rest: bool,
    target: TypeId,
  ) {
    let Some(declared) = self.pattern_defs.get(&pattern.id).copied() else {
      return;
    };
    let name = self.context.def(declared).name.clone();
    let subject = Ident::new(name.clone(), pattern.span);
    let Some((def, args)) = self.named_pattern_subject(target, pattern.span, &subject) else {
      return;
    };
    if def != declared {
      self.report_pattern_subject_mismatch(declared, target, pattern.span);
      return;
    }
    let Some(structure) = self.context.def(def).as_struct().cloned() else {
      self.report(
        CompileError::at(
          ErrorCode::NotAStruct,
          pattern.span,
          format!("`{name}` is not a struct, so it has no fields"),
        )
        .with_help("match an enum variant with `Enum.Variant`"),
      );
      return;
    };

    let mut covered = vec![false; structure.fields.len()];
    for field in fields {
      let Some(index) = structure.field_index(&field.name.name) else {
        let names: Vec<&str> = structure
          .fields
          .iter()
          .map(|entry| entry.name.as_str())
          .collect();
        self.report(
          CompileError::at(
            ErrorCode::UnknownStructField,
            field.name.span,
            format!("`{name}` has no field `{}`", field.name.name),
          )
          .with_note(format!("its fields are {}", join_names(&names))),
        );
        if let Some(inner) = &field.pattern {
          self.check_pattern(inner, TypeId::ERROR);
        }
        continue;
      };
      if covered[index] {
        self.report(CompileError::at(
          ErrorCode::DuplicateField,
          field.name.span,
          format!("`{}` is matched twice in this pattern", field.name.name),
        ));
      }
      covered[index] = true;

      let declared_field = structure.fields[index].clone();
      self.check_field_visibility(def, &declared_field, field.name.span);
      let ty = self
        .context
        .substitute(declared_field.ty, GenericOwner::Type(def), &args);
      match &field.pattern {
        Some(inner) => self.check_pattern(inner, ty),
        None => {
          self.bind_local(&field.name, ty);
        }
      }
    }

    if !rest {
      let missing: Vec<&str> = structure
        .fields
        .iter()
        .zip(&covered)
        .filter(|(_, seen)| !**seen)
        .map(|(field, _)| field.name.as_str())
        .collect();
      if !missing.is_empty() {
        self.report(
          CompileError::at(
            ErrorCode::MissingStructField,
            pattern.span,
            format!("this pattern does not mention {}", join_names(&missing)),
          )
          .with_help("list the remaining fields, or end the pattern with `..`"),
        );
      }
    }
  }

  fn check_tuple_pattern(&mut self, pattern: &Pattern, elements: &[Pattern], target: TypeId) {
    match self.context.kind(target).clone() {
      TypeKind::Tuple(parts) if parts.len() == elements.len() => {
        for (element, part) in elements.iter().zip(parts) {
          self.check_pattern(element, part);
        }
      }
      TypeKind::Error => {
        for element in elements {
          self.check_pattern(element, TypeId::ERROR);
        }
      }
      _ => {
        let shown = self.show(target);
        self.report(CompileError::at(
          ErrorCode::PatternTypeMismatch,
          pattern.span,
          format!(
            "this pattern matches a {}-tuple, but the value is `{shown}`",
            elements.len()
          ),
        ));
        for element in elements {
          self.check_pattern(element, TypeId::ERROR);
        }
      }
    }
  }

  fn check_or_pattern(&mut self, alternatives: &[Pattern], expected: TypeId) {
    let mut reference: Option<(Span, Vec<(String, String)>)> = None;
    for alternative in alternatives {
      self.check_pattern(alternative, expected);
      let bindings = self.pattern_binding_types(alternative);
      match &reference {
        None => reference = Some((alternative.span, bindings)),
        Some((first_span, first)) => {
          if bindings != *first {
            let first_span = *first_span;
            let expected = describe_bindings(first);
            let found = describe_bindings(&bindings);
            self.report(
              CompileError::at(
                ErrorCode::OrPatternBindingMismatch,
                alternative.span,
                format!("this alternative binds {found}"),
              )
              .with_secondary(first_span, format!("this one binds {expected}"))
              .with_note(
                "every alternative of an or-pattern must bind the same names \
                 at the same types",
              ),
            );
            return;
          }
        }
      }
    }
  }

  fn named_pattern_subject(
    &mut self,
    target: TypeId,
    span: Span,
    subject: &Ident,
  ) -> Option<(DefId, Vec<TypeId>)> {
    match self.context.kind(target).clone() {
      TypeKind::Named { def, args } => Some((def, args)),
      TypeKind::Error | TypeKind::Never => None,
      _ => {
        let shown = self.show(target);
        self.report(CompileError::at(
          ErrorCode::PatternTypeMismatch,
          span,
          format!(
            "this pattern matches a `{}`, but the value is `{shown}`",
            subject.name
          ),
        ));
        None
      }
    }
  }

  fn report_pattern_subject_mismatch(&mut self, declared: DefId, target: TypeId, span: Span) {
    let wanted = self.context.def(declared).name.clone();
    let found = self.show(target);
    self.report(CompileError::at(
      ErrorCode::PatternTypeMismatch,
      span,
      format!("this pattern matches a `{wanted}`, but the value is `{found}`"),
    ));
  }

  fn pattern_binding_types(&self, pattern: &Pattern) -> Vec<(String, String)> {
    let mut names = Vec::new();
    collect_pattern_bindings(pattern, &mut names);
    let mut out: Vec<(String, String)> = names
      .into_iter()
      .map(|name| {
        let ty = self
          .declared_locals
          .get(&name.span)
          .map(|local| self.local_types[local.index()])
          .unwrap_or(TypeId::ERROR);
        (name.name.clone(), self.show(ty))
      })
      .collect();
    out.sort();
    out
  }
}

fn covers_the_whole_optional(kind: &PatternKind) -> bool {
  matches!(
    kind,
    PatternKind::Wildcard | PatternKind::Binding(_) | PatternKind::Null
  )
}

fn endpoint_value(endpoint: RangeEndpoint) -> i128 {
  match endpoint {
    RangeEndpoint::Int {
      magnitude,
      negative,
    } => {
      if negative {
        -(magnitude as i128)
      } else {
        magnitude as i128
      }
    }
    RangeEndpoint::Char(value) => value as i128,
  }
}

fn collect_pattern_bindings<'p>(pattern: &'p Pattern, out: &mut Vec<&'p Ident>) {
  match &pattern.kind {
    PatternKind::Binding(name) => out.push(name),
    PatternKind::Variant { payload, .. } => {
      for element in payload.iter().flatten() {
        collect_pattern_bindings(element, out);
      }
    }
    PatternKind::Struct { fields, .. } => {
      for field in fields {
        match &field.pattern {
          Some(inner) => collect_pattern_bindings(inner, out),
          None => out.push(&field.name),
        }
      }
    }
    PatternKind::Tuple(elements) => {
      for element in elements {
        collect_pattern_bindings(element, out);
      }
    }
    PatternKind::Or(alternatives) => {
      if let Some(first) = alternatives.first() {
        collect_pattern_bindings(first, out);
      }
    }
    _ => {}
  }
}

fn describe_bindings(bindings: &[(String, String)]) -> String {
  if bindings.is_empty() {
    return "nothing".to_string();
  }
  let rendered: Vec<String> = bindings
    .iter()
    .map(|(name, ty)| format!("`{name}: {ty}`"))
    .collect();
  rendered.join(", ")
}

fn join_names(names: &[&str]) -> String {
  match names {
    [] => "none".to_string(),
    [only] => format!("`{only}`"),
    [rest @ .., last] => {
      let head: Vec<String> = rest.iter().map(|name| format!("`{name}`")).collect();
      format!("{} and `{last}`", head.join(", "))
    }
  }
}

#[derive(Clone, Debug)]
struct Deconstructed {
  ctor: Ctor,
  fields: Vec<Deconstructed>,
  ty: TypeId,
}

impl Deconstructed {
  fn wildcard(ty: TypeId) -> Deconstructed {
    Deconstructed {
      ctor: Ctor::Wildcard,
      fields: Vec::new(),
      ty,
    }
  }

  fn leaf(ctor: Ctor, ty: TypeId) -> Deconstructed {
    Deconstructed {
      ctor,
      fields: Vec::new(),
      ty,
    }
  }
}

#[derive(Clone, PartialEq, Debug)]
enum Ctor {
  Wildcard,
  Bool(bool),
  Int(i128),
  Char(char),
  Str(String),
  Opaque(u32),
  Variant(u32),
  Single,
  Null,
  Present,
}

enum CtorSet {
  Closed(Vec<Ctor>),
  Open,
}

impl Checker<'_> {
  fn deconstruct(&mut self, pattern: &Pattern, expected: TypeId) -> Vec<Deconstructed> {
    let ty = self.context.shallow_resolve(expected);
    if let TypeKind::Optional(inner) = *self.context.kind(ty) {
      if !covers_the_whole_optional(&pattern.kind) {
        return self
          .deconstruct(pattern, inner)
          .into_iter()
          .map(|field| Deconstructed {
            ctor: Ctor::Present,
            fields: vec![field],
            ty,
          })
          .collect();
      }
    }

    match &pattern.kind {
      PatternKind::Wildcard | PatternKind::Binding(_) => vec![Deconstructed::wildcard(ty)],
      PatternKind::Null => vec![Deconstructed::leaf(Ctor::Null, ty)],
      PatternKind::Bool(value) => vec![Deconstructed::leaf(Ctor::Bool(*value), ty)],
      PatternKind::Int {
        magnitude,
        negative,
      } => {
        let value = if *negative {
          -(*magnitude as i128)
        } else {
          *magnitude as i128
        };
        vec![Deconstructed::leaf(Ctor::Int(value), ty)]
      }
      PatternKind::Char(value) => vec![Deconstructed::leaf(Ctor::Char(*value), ty)],
      PatternKind::Str(value) => vec![Deconstructed::leaf(Ctor::Str(value.clone()), ty)],
      PatternKind::Range { .. } => {
        vec![Deconstructed::leaf(Ctor::Opaque(pattern.id.0), ty)]
      }
      PatternKind::Tuple(elements) => {
        let parts = self.ctor_fields(&Ctor::Single, ty);
        if parts.len() != elements.len() {
          return vec![Deconstructed::wildcard(ty)];
        }
        self.deconstruct_fields(Ctor::Single, ty, elements.iter().zip(parts).collect())
      }
      PatternKind::Variant {
        variant, payload, ..
      } => {
        let TypeKind::Named { def, .. } = *self.context.kind(ty) else {
          return vec![Deconstructed::wildcard(ty)];
        };
        let Some(index) = self
          .context
          .def(def)
          .as_enum()
          .and_then(|enumeration| enumeration.variant_index(&variant.name))
        else {
          return vec![Deconstructed::wildcard(ty)];
        };
        let ctor = Ctor::Variant(index as u32);
        let parts = self.ctor_fields(&ctor, ty);
        let elements: Vec<&Pattern> = payload.iter().flatten().collect();
        if elements.len() != parts.len() {
          return vec![Deconstructed::wildcard(ty)];
        }
        self.deconstruct_fields(ctor, ty, elements.into_iter().zip(parts).collect())
      }
      PatternKind::Struct { fields, .. } => {
        let TypeKind::Named { def, .. } = *self.context.kind(ty) else {
          return vec![Deconstructed::wildcard(ty)];
        };
        let Some(structure) = self.context.def(def).as_struct().cloned() else {
          return vec![Deconstructed::wildcard(ty)];
        };
        let parts = self.ctor_fields(&Ctor::Single, ty);
        if parts.len() != structure.fields.len() {
          return vec![Deconstructed::wildcard(ty)];
        }
        // truong bi bo, du la do `..` hay do pattern viet hong, deu
        // tinh la mot cot wildcard
        let mut columns: Vec<Vec<Deconstructed>> = parts
          .iter()
          .map(|&part| vec![Deconstructed::wildcard(part)])
          .collect();
        for field in fields {
          let Some(index) = structure.field_index(&field.name.name) else {
            continue;
          };
          columns[index] = match &field.pattern {
            Some(inner) => self.deconstruct(inner, parts[index]),
            None => vec![Deconstructed::wildcard(parts[index])],
          };
        }
        combine(Ctor::Single, ty, columns)
      }
      PatternKind::Or(alternatives) => {
        let mut out = Vec::new();
        for alternative in alternatives {
          out.extend(self.deconstruct(alternative, ty));
          if out.len() > EXPANSION_LIMIT {
            return vec![Deconstructed::wildcard(ty)];
          }
        }
        out
      }
    }
  }

  fn deconstruct_fields(
    &mut self,
    ctor: Ctor,
    ty: TypeId,
    pairs: Vec<(&Pattern, TypeId)>,
  ) -> Vec<Deconstructed> {
    let columns: Vec<Vec<Deconstructed>> = pairs
      .into_iter()
      .map(|(pattern, part)| self.deconstruct(pattern, part))
      .collect();
    combine(ctor, ty, columns)
  }

  fn ctor_set(&self, ty: TypeId) -> CtorSet {
    let ty = self.context.shallow_resolve(ty);
    match self.context.kind(ty) {
      TypeKind::Error | TypeKind::Never => CtorSet::Closed(Vec::new()),
      TypeKind::Bool => CtorSet::Closed(vec![Ctor::Bool(false), Ctor::Bool(true)]),
      TypeKind::Optional(_) => CtorSet::Closed(vec![Ctor::Null, Ctor::Present]),
      TypeKind::Tuple(_) => CtorSet::Closed(vec![Ctor::Single]),
      TypeKind::Named { def, .. } => match &self.context.def(*def).obj_kind {
        crate::types::TypeDefKind::Struct(_) => CtorSet::Closed(vec![Ctor::Single]),
        crate::types::TypeDefKind::Enum(enumeration) => CtorSet::Closed(
          (0..enumeration.variants.len())
            .map(|index| Ctor::Variant(index as u32))
            .collect(),
        ),
        crate::types::TypeDefKind::Interface(_) => CtorSet::Open,
      },
      _ => CtorSet::Open,
    }
  }

  fn ctor_fields(&mut self, ctor: &Ctor, ty: TypeId) -> Vec<TypeId> {
    let ty = self.context.shallow_resolve(ty);
    match ctor {
      Ctor::Present => match *self.context.kind(ty) {
        TypeKind::Optional(inner) => vec![inner],
        _ => Vec::new(),
      },
      Ctor::Single => match self.context.kind(ty).clone() {
        TypeKind::Tuple(parts) => parts,
        TypeKind::Named { def, args } => {
          let Some(structure) = self.context.def(def).as_struct().cloned() else {
            return Vec::new();
          };
          structure
            .fields
            .iter()
            .map(|field| {
              self.context
                .substitute(field.ty, GenericOwner::Type(def), &args)
            })
            .collect()
        }
        _ => Vec::new(),
      },
      Ctor::Variant(index) => {
        let TypeKind::Named { def, args } = self.context.kind(ty).clone() else {
          return Vec::new();
        };
        let Some(enumeration) = self.context.def(def).as_enum().cloned() else {
          return Vec::new();
        };
        let Some(variant) = enumeration.variants.get(*index as usize) else {
          return Vec::new();
        };
        variant
          .payload
          .clone()
          .into_iter()
          .map(|payload| {
            self.context
              .substitute(payload, GenericOwner::Type(def), &args)
          })
          .collect()
      }
      _ => Vec::new(),
    }
  }

  fn specialise(
    &mut self,
    matrix: &[Vec<Deconstructed>],
    ctor: &Ctor,
    fields: &[TypeId],
  ) -> Vec<Vec<Deconstructed>> {
    let mut out = Vec::new();
    for row in matrix {
      let Some((head, rest)) = row.split_first() else {
        continue;
      };
      if head.ctor == Ctor::Wildcard {
        let mut expanded: Vec<Deconstructed> = fields
          .iter()
          .map(|&ty| Deconstructed::wildcard(ty))
          .collect();
        expanded.extend_from_slice(rest);
        out.push(expanded);
      } else if &head.ctor == ctor {
        let mut expanded = head.fields.clone();
        expanded.extend_from_slice(rest);
        out.push(expanded);
      }
    }
    out
  }

  fn is_useful(&mut self, matrix: &[Vec<Deconstructed>], row: &[Deconstructed]) -> bool {
    let Some((head, rest)) = row.split_first() else {
      return matrix.is_empty();
    };
    let ty = head.ty;

    if head.ctor != Ctor::Wildcard {
      let ctor = head.ctor.clone();
      let fields = self.ctor_fields(&ctor, ty);
      let specialised = self.specialise(matrix, &ctor, &fields);
      let mut query = head.fields.clone();
      query.extend_from_slice(rest);
      return self.is_useful(&specialised, &query);
    }

    let used = hea_cto(matrix);
    match self.ctor_set(ty) {
      // khong co gi de suy luan. Dung bao gio ket luan mot nhanh khong
      // toi duoc dua ...
      CtorSet::Closed(all) if all.is_empty() => true,
      CtorSet::Closed(all) if all.iter().all(|ctor| used.contains(ctor)) => {
        for ctor in all {
          let fields = self.ctor_fields(&ctor, ty);
          let specialised = self.specialise(matrix, &ctor, &fields);
          let mut query: Vec<Deconstructed> = fields
            .iter()
            .map(|&ty| Deconstructed::wildcard(ty))
            .collect();
          query.extend_from_slice(rest);
          if self.is_useful(&specialised, &query) {
            return true;
          }
        }
        false
      }
      _ => {
        let defaulted = default_matrix(matrix);
        self.is_useful(&defaulted, rest)
      }
    }
  }

  fn witnesses(
    &mut self,
    matrix: &[Vec<Deconstructed>],
    row: &[Deconstructed],
  ) -> Vec<Vec<WitnessPat>> {
    let Some((head, rest)) = row.split_first() else {
      return if matrix.is_empty() {
        vec![Vec::new()]
      } else {
        Vec::new()
      };
    };
    let ty = head.ty;
    let used = hea_cto(matrix);
    let set = self.ctor_set(ty);

    let (missing, open) = match &set {
      CtorSet::Closed(all) => (
        all.iter()
          .filter(|ctor| !used.contains(ctor))
          .cloned()
          .collect::<Vec<Ctor>>(),
        false,
      ),
      CtorSet::Open => (Vec::new(), true),
    };

    if !open && missing.is_empty() {
      let CtorSet::Closed(all) = set else {
        unreachable!("the open case was handled above")
      };
      let mut out = Vec::new();
      for ctor in all {
        let fields = self.ctor_fields(&ctor, ty);
        let specialised = self.specialise(matrix, &ctor, &fields);
        let mut query: Vec<Deconstructed> = fields
          .iter()
          .map(|&ty| Deconstructed::wildcard(ty))
          .collect();
        query.extend_from_slice(rest);
        for witness in self.witnesses(&specialised, &query) {
          out.push(self.apply_ctor(&ctor, ty, witness));
          if out.len() >= WITNESS_LIMIT * 2 {
            return out;
          }
        }
      }
      return out;
    }

    let defaulted = default_matrix(matrix);
    let tails = self.witnesses(&defaulted, rest);
    if tails.is_empty() {
      return Vec::new();
    }
    let heads: Vec<WitnessPat> = if open {
      vec![WitnessPat::Wildcard]
    } else {
      missing
        .iter()
        .map(|ctor| self.witness_of(ctor, ty))
        .collect()
    };

    let mut out = Vec::new();
    for tail in &tails {
      for head in &heads {
        let mut full = vec![head.clone()];
        full.extend(tail.iter().cloned());
        out.push(full);
        if out.len() >= WITNESS_LIMIT * 2 {
          return out;
        }
      }
    }
    out
  }

  fn apply_ctor(
    &mut self,
    ctor: &Ctor,
    ty: TypeId,
    mut witness: Vec<WitnessPat>,
  ) -> Vec<WitnessPat> {
    let arity = self.ctor_fields(ctor, ty).len();
    let rest = witness.split_off(arity.min(witness.len()));
    let mut fields = witness;
    while fields.len() < arity {
      fields.push(WitnessPat::Wildcard);
    }
    let mut out = vec![self.build_witness(ctor, ty, fields)];
    out.extend(rest);
    out
  }

  fn witness_of(&mut self, ctor: &Ctor, ty: TypeId) -> WitnessPat {
    let arity = self.ctor_fields(ctor, ty).len();
    self.build_witness(ctor, ty, vec![WitnessPat::Wildcard; arity])
  }

  fn build_witness(&self, ctor: &Ctor, ty: TypeId, fields: Vec<WitnessPat>) -> WitnessPat {
    let ty = self.context.shallow_resolve(ty);
    match ctor {
      Ctor::Wildcard | Ctor::Opaque(_) => WitnessPat::Wildcard,
      Ctor::Bool(value) => WitnessPat::Bool(*value),
      Ctor::Int(value) => WitnessPat::Int(*value),
      Ctor::Char(value) => WitnessPat::Char(*value),
      Ctor::Str(value) => WitnessPat::Str(value.clone()),
      Ctor::Null => WitnessPat::Null,
      Ctor::Present => WitnessPat::Present(Box::new(
        fields.into_iter().next().unwrap_or(WitnessPat::Wildcard),
      )),
      Ctor::Variant(index) => match *self.context.kind(ty) {
        TypeKind::Named { def, .. } => WitnessPat::Variant {
          def,
          variant: *index,
          fields,
        },
        _ => WitnessPat::Wildcard,
      },
      Ctor::Single => match *self.context.kind(ty) {
        TypeKind::Tuple(_) => WitnessPat::Tuple(fields),
        TypeKind::Named { def, .. } => WitnessPat::Struct { def, fields },
        _ => WitnessPat::Wildcard,
      },
    }
  }

  fn render_witness(&self, witness: &WitnessPat) -> String {
    match witness {
      WitnessPat::Wildcard => "_".to_string(),
      WitnessPat::Bool(value) => value.to_string(),
      WitnessPat::Int(value) => value.to_string(),
      WitnessPat::Char(value) => format!("'{value}'"),
      WitnessPat::Str(value) => format!("\"{value}\""),
      WitnessPat::Null => "null".to_string(),
      WitnessPat::Present(inner) => self.render_witness(inner),
      WitnessPat::Tuple(fields) => {
        let rendered: Vec<String> = fields
          .iter()
          .map(|field| self.render_witness(field))
          .collect();
        format!("({})", rendered.join(", "))
      }
      WitnessPat::Variant {
        def,
        variant,
        fields,
      } => {
        let owner = &self.context.def(*def).name;
        let name = self
          .context
          .def(*def)
          .as_enum()
          .and_then(|enumeration| enumeration.variants.get(*variant as usize))
          .map(|entry| entry.name.as_str())
          .unwrap_or("?");
        if fields.is_empty() {
          return format!("{owner}.{name}");
        }
        let rendered: Vec<String> = fields
          .iter()
          .map(|field| self.render_witness(field))
          .collect();
        format!("{owner}.{name}({})", rendered.join(", "))
      }
      WitnessPat::Struct { def, fields } => {
        let owner = &self.context.def(*def).name;
        let Some(structure) = self.context.def(*def).as_struct() else {
          return format!("{owner} {{ .. }}");
        };
        let interesting: Vec<String> = structure
          .fields
          .iter()
          .zip(fields)
          .filter(|(_, witness)| !matches!(witness, WitnessPat::Wildcard))
          .map(|(field, witness)| {
            format!("{}: {}", field.name, self.render_witness(witness))
          })
          .collect();
        if interesting.is_empty() {
          format!("{owner} {{ .. }}")
        } else {
          format!("{owner} {{ {}, .. }}", interesting.join(", "))
        }
      }
    }
  }
}

#[derive(Clone, Debug)]
enum WitnessPat {
  Wildcard,
  Bool(bool),
  Int(i128),
  Char(char),
  Str(String),
  Null,
  Present(Box<WitnessPat>),
  Variant {
    def: DefId,
    variant: u32,
    fields: Vec<WitnessPat>,
  },
  Struct {
    def: DefId,
    fields: Vec<WitnessPat>,
  },
  Tuple(Vec<WitnessPat>),
}

fn hea_cto(matrix: &[Vec<Deconstructed>]) -> Vec<Ctor> {
  let mut out: Vec<Ctor> = Vec::new();
  for row in matrix {
    let Some(head) = row.first() else { continue };
    if head.ctor != Ctor::Wildcard && !out.contains(&head.ctor) {
      out.push(head.ctor.clone());
    }
  }
  out
}

fn default_matrix(matrix: &[Vec<Deconstructed>]) -> Vec<Vec<Deconstructed>> {
  matrix
    .iter()
    .filter_map(|row| {
      let (head, rest) = row.split_first()?;
      (head.ctor == Ctor::Wildcard).then(|| rest.to_vec())
    })
    .collect()
}

fn combine(ctor: Ctor, ty: TypeId, columns: Vec<Vec<Deconstructed>>) -> Vec<Deconstructed> {
  let total: usize = columns.iter().map(|column| column.len().max(1)).product();
  if total > EXPANSION_LIMIT {
    return vec![Deconstructed::wildcard(ty)];
  }
  let mut rows: Vec<Vec<Deconstructed>> = vec![Vec::new()];
  for column in columns {
    let mut next = Vec::with_capacity(rows.len() * column.len().max(1));
    for row in &rows {
      for entry in &column {
        let mut extended = row.clone();
        extended.push(entry.clone());
        next.push(extended);
      }
    }
    rows = next;
  }
  rows.into_iter()
    .map(|fields| Deconstructed {
      ctor: ctor.clone(),
      fields,
      ty,
    })
    .collect()
}
