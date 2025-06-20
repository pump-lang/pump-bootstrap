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
      // toi duoc dua tren mot kieu von da check hong.
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

impl Checker<'_> {
  fn check_field_expr(
    &mut self,
    expr: &Expr,
    base: &Expr,
    name: &Ident,
    expected: Option<TypeId>,
  ) -> TypeId {
    // duong dan co ten module dang truoc: resolver da buoc ca cum `a.b`,
    // vi no la ...
    if let Some(binding) = self.values.get(&expr.id).copied() {
      return self.module_member_type(binding, name);
    }
    // `Enum.Variant`: goc la ten mot kieu, nen `b` la variant (16.7)
    if let Some(def) = self.type_path_base(base) {
      return self.check_variant_reference(def, expr.id, name, expected);
    }
    let receiver = self.check_value(base, None);
    let member = self.member_of(base, receiver, name);
    self.member_value_type(expr.id, member, name)
  }

  fn module_member_type(&mut self, binding: ValueBinding, name: &Ident) -> TypeId {
    match binding {
      ValueBinding::Function(func) => self.function_value_type(func, name.span),
      ValueBinding::GlobalConst(global) => self.global_types[global.index()],
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
      _ => TypeId::ERROR,
    }
  }

  fn check_variant_reference(
    &mut self,
    def: DefId,
    node: NodeId,
    name: &Ident,
    expected: Option<TypeId>,
  ) -> TypeId {
    let owner = self.context.def(def).name.clone();
    let Some(enumeration) = self.context.def(def).as_enum().cloned() else {
      self.report(
        CompileError::at(
          ErrorCode::NotAnEnum,
          name.span,
          format!(
            "`{owner}` is not an enum, so `{owner}.{}` names nothing",
            name.name
          ),
        )
        .with_help("a struct value is built with `Type { ... }`"),
      );
      return TypeId::ERROR;
    };
    let Some(index) = enumeration.variant_index(&name.name) else {
      let names: Vec<&str> = enumeration
        .variants
        .iter()
        .map(|variant| variant.name.as_str())
        .collect();
      self.report(
        CompileError::at(
          ErrorCode::UnknownVariant,
          name.span,
          format!("`{owner}` has no variant `{}`", name.name),
        )
        .with_note(format!("its variants are {}", join_names(&names))),
      );
      return TypeId::ERROR;
    };

    let args = self.enum_arguments(def, expected);
    if !enumeration.variants[index].payload.is_empty() {
      let arity = enumeration.variants[index].payload.len();
      self.report(
        CompileError::at(
          ErrorCode::WrongArgumentCount,
          name.span,
          format!(
            "`{owner}.{}` carries {arity} value{}, so it must be applied to {}",
            name.name,
            if arity == 1 { "" } else { "s" },
            if arity == 1 { "it" } else { "them" }
          ),
        )
        .with_help(format!("write `{owner}.{}(...)`", name.name)),
      );
      return TypeId::ERROR;
    }

    self.constants.insert(
      node,
      ConstValue::EnumVariant {
        def,
        variant: index as u32,
      },
    );
    let ty = self.context.named(def, args);
    self.demand_inferred(ty, name.span, &format!("`{owner}.{}`", name.name));
    ty
  }

  fn type_path_base(&self, receiver: &Expr) -> Option<DefId> {
    if !matches!(receiver.kind, ExprKind::Ident(_) | ExprKind::Field { .. }) {
      return None;
    }
    match self.values.get(&receiver.id)? {
      ValueBinding::Type(def) => Some(*def),
      _ => None,
    }
  }

  fn enum_arguments(&mut self, def: DefId, expected: Option<TypeId>) -> Vec<TypeId> {
    let arity = self.context.def(def).generics.len();
    if arity == 0 {
      return Vec::new();
    }
    if let Some(hint) = self.literal_expectation(expected) {
      if let TypeKind::Named {
        def: other,
        ref args,
      } = *self.context.kind(hint)
      {
        if other == def && args.len() == arity {
          return args.clone();
        }
      }
    }
    (0..arity).map(|_| self.context.fresh_var()).collect()
  }

  fn member_value_type(&mut self, node: NodeId, member: Member, name: &Ident) -> TypeId {
    match member {
      Member::Field { owner, index, ty } => {
        self.field_accesses
          .insert(node, FieldAccess::Field { owner, index });
        ty
      }
      Member::Length => {
        self.field_accesses.insert(node, FieldAccess::Length);
        TypeId::INT
      }
      Member::Method { .. } | Member::InterfaceMethod { .. } | Member::Builtin { .. } => {
        self.report(
          CompileError::at(
            ErrorCode::UnknownField,
            name.span,
            format!("`{}` is a method and must be called", name.name),
          )
          .with_help(format!("write `{}()`", name.name))
          .with_note("Pump 1.0 has no method values; wrap it in a closure instead"),
        );
        TypeId::ERROR
      }
      Member::Unknown => TypeId::ERROR,
    }
  }

  fn member_of(&mut self, base: &Expr, receiver: TypeId, name: &Ident) -> Member {
    let receiver = self.context.shallow_resolve(receiver);
    if matches!(
      self.context.kind(receiver),
      TypeKind::Error | TypeKind::Never
    ) {
      return Member::Unknown;
    }
    if name.name == "length" && self.has_length(receiver) {
      return Member::Length;
    }
    if let Some(member) = self.builtin_member(receiver, &name.name) {
      return member;
    }

    match self.context.kind(receiver).clone() {
      TypeKind::Named { def, args } => self.named_member(def, args, receiver, name),
      TypeKind::Generic(generic) => self.generic_member(generic, name),
      TypeKind::Optional(inner) => {
        let shown = self.show(inner);
        let error = CompileError::at(
          ErrorCode::UnknownField,
          name.span,
          format!(
            "`{shown}?` may be absent, so `.{}` is not available",
            name.name
          ),
        );
        // Neu goc la mot bien DA duoc thu hep roi mat thu hep vi mot
        // phep gan (16.10) thi bao "test it with `!= null`" la vo
        // duyen: nguoi ta test roi. Phai chi vao chinh cho gan.
        self.report(match self.defeated_at(base) {
          Some(at) => error
            .with_secondary(at, "this assignment ends the narrowing")
            .with_help("copy it into a new binding inside the region and use that")
            .with_note(
              "16.10: a narrowing covers a whole region, and an assignment to the narrowed name anywhere in that region defeats it",
            ),
          None => error.with_help(
            "test it with `!= null`, propagate with `?`, or use `.expect(\"...\")`",
          ),
        });
        Member::Unknown
      }
      TypeKind::Tuple(_) => {
        self.report(
          CompileError::at(
            ErrorCode::UnknownField,
            name.span,
            "a tuple has no named members",
          )
          .with_help("index it by position: `t.0`"),
        );
        Member::Unknown
      }
      _ => {
        let shown = self.show(receiver);
        self.report(CompileError::at(
          ErrorCode::UnknownField,
          name.span,
          format!("`{shown}` has no member `{}`", name.name),
        ));
        Member::Unknown
      }
    }
  }

  fn named_member(
    &mut self,
    def: DefId,
    args: Vec<TypeId>,
    receiver: TypeId,
    name: &Ident,
  ) -> Member {
    let definition = self.context.def(def).clone();
    // obj = definition, chua doi het ten
    let obj = definition.clone();
    // `receiver` la kieu ma method se
    // chu khong phai kieu khai bao tran.
    match &obj.obj_kind {
      crate::types::TypeDefKind::Struct(structure) => {
        if let Some(index) = structure.field_index(&name.name) {
          let field = structure.fields[index].clone();
          self.check_field_visibility(def, &field, name.span);
          let ty = self
            .context
            .substitute(field.ty, GenericOwner::Type(def), &args);
          return Member::Field {
            owner: def,
            index: index as u32,
            ty,
          };
        }
        if let Some(func) = self.context.find_method(def, &name.name) {
          self.check_method_visibility(func, name.span);
          return Member::Method {
            owner: def,
            func,
            receiver,
          };
        }
        let fields: Vec<&str> = structure
          .fields
          .iter()
          .map(|field| field.name.as_str())
          .collect();
        let methods: Vec<String> = structure
          .methods
          .iter()
          .map(|&func| self.context.func(func).name.clone())
          .collect();
        let method_names: Vec<&str> = methods.iter().map(|entry| entry.as_str()).collect();
        let owner = definition.name.clone();
        self.report(
          CompileError::at(
            ErrorCode::UnknownField,
            name.span,
            format!("`{owner}` has no field or method `{}`", name.name),
          )
          .with_note(format!("its fields are {}", join_names(&fields)))
          .with_note(format!("its methods are {}", join_names(&method_names))),
        );
        Member::Unknown
      }
      crate::types::TypeDefKind::Enum(enumeration) => {
        if let Some(func) = self.context.find_method(def, &name.name) {
          self.check_method_visibility(func, name.span);
          return Member::Method {
            owner: def,
            func,
            receiver,
          };
        }
        let owner = definition.name.clone();
        if enumeration.variant_index(&name.name).is_some() {
          self.report( CompileError::at( ErrorCode::UnknownMethod, name.span, format!( "`{}` is a variant of `{owner}`, not a member of one", name.name ),
            )
            .with_help(format!(
              "name it as `{owner}.{}`, or `match` on the value",
              name.name
            )),
          );
          return Member::Unknown;
        }
        let methods: Vec<String> = enumeration
          .methods
          .iter()
          .map(|&func| self.context.func(func).name.clone())
          .collect();
        let method_names: Vec<&str> = methods.iter().map(|entry| entry.as_str()).collect();
        self.report(
          CompileError::at(
            ErrorCode::UnknownMethod,
            name.span,
            format!("`{owner}` has no method `{}`", name.name),
          )
          .with_note(format!("its methods are {}", join_names(&method_names))),
        );
        Member::Unknown
      }
      crate::types::TypeDefKind::Interface(interface) => {
        let slot = interface
          .methods
          .iter()
          .position(|&func| self.context.func(func).name == name.name);
        match slot {
          Some(slot) => Member::InterfaceMethod {
            interface: def,
            slot: slot as u32,
            func: interface.methods[slot],
          },
          None => {
            let owner = definition.name.clone();
            let methods: Vec<String> = interface
              .methods
              .iter()
              .map(|&func| self.context.func(func).name.clone())
              .collect();
            let method_names: Vec<&str> =
              methods.iter().map(|entry| entry.as_str()).collect();
            self.report(
              CompileError::at( ErrorCode::UnknownMethod, name.span, format!("the interface `{owner}` declares no `{}`", name.name), )
              .with_note(format!("it declares {}", join_names(&method_names))),
            );
            Member::Unknown
          }
        }
      }
    }
  }

  fn generic_parameter(
    &self,
    generic: crate::types::GenericId,
  ) -> Option<crate::types::GenericParamDef> {
    let generics = match generic.owner {
      GenericOwner::Type(def) => &self.context.def(def).generics,
      GenericOwner::Func(func) => &self.context.func(func).generics,
    };
    generics.get(generic.index as usize).cloned()
  }

  fn bound_supplies(&self, generic: crate::types::GenericId, method: &str) -> bool {
    let Some(parameter) = self.generic_parameter(generic) else {
      return false;
    };
    parameter.bounds.iter().any(|bound| {
      self.context
        .def(*bound)
        .as_interface()
        .is_some_and(|interface| {
          interface
            .methods
            .iter()
            .any(|func| self.context.func(*func).name == method)
        })
    })
  }

  fn generic_member(&mut self, generic: crate::types::GenericId, name: &Ident) -> Member {
    let Some(parameter) = self.generic_parameter(generic) else {
      return Member::Unknown;
    };
    if parameter.bounds.is_empty() {
      self.report(
        CompileError::at(
          ErrorCode::MethodOnUnboundedGeneric,
          name.span,
          format!(
            "`{}` has no interface bounds, so `.{}` is not available on it",
            parameter.name, name.name
          ),
        )
        .with_secondary(parameter.span, "declared without a bound here")
        .with_note(
          "an unbounded type parameter may only be bound, passed, returned and stored",
        )
        .with_help(format!(
          "give it a bound: `<{}: SomeInterface>`",
          parameter.name
        )),
      );
      return Member::Unknown;
    }
    for &interface in &parameter.bounds {
      let methods = self
        .context
        .def(interface)
        .as_interface()
        .map(|declared| declared.methods.clone())
        .unwrap_or_default();
      if let Some(slot) = methods
        .iter()
        .position(|&func| self.context.func(func).name == name.name)
      {
        return Member::InterfaceMethod {
          interface,
          slot: slot as u32,
          func: methods[slot],
        };
      }
    }
    let bounds: Vec<String> = parameter
      .bounds
      .iter()
      .map(|&interface| self.context.def(interface).name.clone())
      .collect();
    let bound_names: Vec<&str> = bounds.iter().map(|entry| entry.as_str()).collect();
    self.report(
      CompileError::at(
        ErrorCode::UnknownMethod,
        name.span,
        format!(
          "none of the bounds on `{}` declares `{}`",
          parameter.name, name.name
        ),
      )
      .with_note(format!("its bounds are {}", join_names(&bound_names))),
    );
    Member::Unknown
  }

  fn builtin_member(&mut self, receiver: TypeId, name: &str) -> Option<Member> {
    use BuiltinMethod::*;
    let (method, params, ret) = match self.context.kind(receiver).clone() {
      TypeKind::String => match name {
        "to_string" => (ToString, Vec::new(), TypeId::STRING),
        "message" => (StringMessage, Vec::new(), TypeId::STRING),
        "chars" => {
          let array = self.context.array_of(TypeId::CHAR);
          (StringChars, Vec::new(), array)
        }
        "char_count" => (StringCharCount, Vec::new(), TypeId::INT),
        "byte_at" => (StringByteAt, vec![TypeId::INT], TypeId::INT),
        "slice" => (StringSlice, vec![TypeId::INT, TypeId::INT], TypeId::STRING),
        _ => return None,
      },
      TypeKind::Array(element) => match name {
        "push" => (ArrayPush, vec![element], TypeId::VOID),
        "pop" => (ArrayPop, Vec::new(), element),
        "slice" => (ArraySlice, vec![TypeId::INT, TypeId::INT], receiver),
        "concat" => (ArrayConcat, vec![receiver], receiver),
        "reserve" => (ArrayReserve, vec![TypeId::INT], TypeId::VOID),
        _ => return None,
      },
      TypeKind::Map { key, value } => match name {
        "has" => (MapHas, vec![key], TypeId::BOOL),
        "get" => {
          // `m[k]` panic khi thieu khoa, con `m.get(k)` la dang day
          // du nen tra ve optional
          let optional = self.context.optional_of(value);
          (MapGet, vec![key], optional)
        }
        "insert" => (MapInsert, vec![key, value], TypeId::VOID),
        "remove" => (MapRemove, vec![key], TypeId::BOOL),
        "keys" => {
          let array = self.context.array_of(key);
          (MapKeys, Vec::new(), array)
        }
        "values" => {
          let array = self.context.array_of(value);
          (MapValues, Vec::new(), array)
        }
        _ => return None,
      },
      TypeKind::Set(element) => match name {
        "add" => (SetAdd, vec![element], TypeId::BOOL),
        "has" => (SetHas, vec![element], TypeId::BOOL),
        "remove" => (SetRemove, vec![element], TypeId::BOOL),
        _ => return None,
      },
      TypeKind::Optional(inner) => match name {
        "expect" => (OptionalExpect, vec![TypeId::STRING], inner),
        "or" => (OptionalOr, vec![inner], inner),
        _ => return None,
      },
      TypeKind::Bool | TypeKind::Int | TypeKind::Uint | TypeKind::Float | TypeKind::Char => {
        match name {
          "to_string" => (ToString, Vec::new(), TypeId::STRING),
          _ => return None,
        }
      }
      _ => return None,
    };
    Some(Member::Builtin {
      method,
      params,
      ret,
    })
  }

  fn has_length(&self, ty: TypeId) -> bool {
    matches!(
      self.context.kind(ty),
      TypeKind::Array(_) | TypeKind::Map { .. } | TypeKind::Set(_) | TypeKind::String
    )
  }

  fn check_field_visibility(&mut self, owner: DefId, field: &crate::types::FieldDef, span: Span) {
    if field.visibility == VisibilityKind::Public {
      return;
    }
    let module = self.context.def(owner).module;
    if module == self.module {
      return;
    }
    let owner_name = self.context.def(owner).name.clone();
    let path = self.context.module_path(module).join("\\");
    let field_name = field.name.clone();
    let declared = field.span;
    self.report(
      CompileError::at(
        ErrorCode::PrivateAccess,
        span,
        format!("the field `{field_name}` of `{owner_name}` is private to module `{path}`"),
      )
      .with_secondary(declared, "declared here")
      .with_help(format!("declare it `pub` in `{path}`")),
    );
  }

  fn check_method_visibility(&mut self, func: FuncId, span: Span) {
    let definition = self.context.func(func);
    if definition.visibility == VisibilityKind::Public || definition.module == self.module {
      return;
    }
    let name = definition.name.clone();
    let module = definition.module;
    let declared = definition.span;
    let path = self.context.module_path(module).join("\\");
    self.report(
      CompileError::at(
        ErrorCode::PrivateAccess,
        span,
        format!("the method `{name}` is private to module `{path}`"),
      )
      .with_secondary(declared, "declared here")
      .with_help(format!("declare it `pub` in `{path}`")),
    );
  }

  fn check_tuple_field(&mut self, base: &Expr, index: u32, index_span: Span) -> TypeId {
    let receiver = self.check_value(base, None);
    let receiver = self.context.shallow_resolve(receiver);
    match self.context.kind(receiver).clone() {
      TypeKind::Error | TypeKind::Never => TypeId::ERROR,
      TypeKind::Tuple(parts) => match parts.get(index as usize) {
        Some(&ty) => ty,
        None => {
          self.report(
            CompileError::at(
              ErrorCode::TupleIndexOutOfRange,
              index_span,
              format!(
                "this tuple has {} element{}, so `.{index}` names nothing",
                parts.len(),
                if parts.len() == 1 { "" } else { "s" }
              ),
            )
            .with_help(format!("the last element is `.{}`", parts.len() - 1)),
          );
          TypeId::ERROR
        }
      },
      _ => {
        let shown = self.show(receiver);
        let mut error = CompileError::at(
          ErrorCode::NotATuple,
          index_span,
          format!("`{shown}` is not a tuple, so `.{index}` names nothing"),
        );
        if matches!(self.context.kind(receiver), TypeKind::Array(_)) {
          error = error.with_help(format!("index an array with brackets: `a[{index}]`"));
        }
        self.report(error);
        TypeId::ERROR
      }
    }
  }

  fn check_index(&mut self, base: &Expr, index: &Expr, span: Span) -> TypeId {
    let receiver = self.check_value(base, None);
    let receiver = self.context.shallow_resolve(receiver);
    match self.context.kind(receiver).clone() {
      TypeKind::Error | TypeKind::Never => {
        self.check_value(index, None);
        TypeId::ERROR
      }
      TypeKind::Array(element) => {
        let key = self.check_value(index, Some(TypeId::INT));
        self.expect_assignable(TypeId::INT, key, index.span, "an array index");
        element
      }
      TypeKind::Map { key, value } => {
        let supplied = self.check_value(index, Some(key));
        self.expect_assignable(key, supplied, index.span, "a map key");
        value
      }
      TypeKind::String => {
        self.check_value(index, None);
        self.report(
          CompileError::at(
            ErrorCode::StringNotIndexable,
            span,
            "a `string` is UTF-8, so a byte offset is not a character",
          )
          .with_help("iterate it with `for c in s`, or use `s.byte_at(i)`"),
        );
        TypeId::ERROR
      }
      TypeKind::Set(_) => {
        self.check_value(index, None);
        self.report(
          CompileError::at(
            ErrorCode::NotIndexable,
            span,
            "a set has no positions to index",
          )
          .with_help("test membership with `s.has(x)`"),
        );
        TypeId::ERROR
      }
      TypeKind::Tuple(_) => {
        self.check_value(index, None);
        self.report(
          CompileError::at(
            ErrorCode::NotIndexable,
            span,
            "a tuple is indexed by a constant position, not by a value",
          )
          .with_help("write `t.0`"),
        );
        TypeId::ERROR
      }
      TypeKind::Optional(_) => {
        self.check_value(index, None);
        let shown = self.show(receiver);
        self.report(
          CompileError::at(
            ErrorCode::NotIndexable,
            span,
            format!("`{shown}` may be absent, so it cannot be indexed"),
          )
          .with_help("test it against `null` first"),
        );
        TypeId::ERROR
      }
      _ => {
        self.check_value(index, None);
        let shown = self.show(receiver);
        self.report(
          CompileError::at(
            ErrorCode::NotIndexable,
            span,
            format!("`{shown}` cannot be indexed"),
          )
          .with_note("Pump 1.0 indexes arrays and maps"),
        );
        TypeId::ERROR
      }
    }
  }

  fn check_null_propagate(&mut self, operand: &Expr, span: Span) -> TypeId {
    let ty = self.check_value(operand, None);
    let resolved = self.context.shallow_resolve(ty);
    let TypeKind::Optional(inner) = *self.context.kind(resolved) else {
      if !matches!(
        self.context.kind(resolved),
        TypeKind::Error | TypeKind::Never
      ) {
        let shown = self.show(resolved);
        self.report(
          CompileError::at(
            ErrorCode::TypeMismatch,
            span,
            format!("`?` propagates a `null`, but `{shown}` is never absent"),
          )
          .with_help("drop the `?`"),
        );
      }
      return resolved;
    };
    let ret = self.context.shallow_resolve(self.signature.ret);
    if !matches!(
      self.context.kind(ret),
      TypeKind::Optional(_) | TypeKind::Error
    ) {
      let shown = self.show(ret);
      let inner_shown = self.show(inner);
      self.report(
        CompileError::at(
          ErrorCode::PropagateNullInNonOptional,
          span,
          format!("`?` returns `null` from a function that returns `{shown}`"),
        )
        .with_help(format!("declare the return type as `{shown}?`"))
        .with_note(format!(
          "or handle the absence here: `.expect(\"...\")` or `.or(<{inner_shown}>)`"
        )),
      );
    }
    inner
  }

  fn check_error_propagate(&mut self, operand: &Expr, span: Span) -> TypeId {
    // co y khong dung `check_value`: `!` la mot trong hai thu duoc phep
    // nuot mot bieu thuc co the loi
    let ty = self.check_expr(operand, None);
    let resolved = self.context.shallow_resolve(ty);
    let TypeKind::Failable(inner) = *self.context.kind(resolved) else {
      if !matches!(
        self.context.kind(resolved),
        TypeKind::Error | TypeKind::Never
      ) {
        let shown = self.show(resolved);
        self.report(
          CompileError::at(
            ErrorCode::TypeMismatch,
            span,
            format!("`!` propagates a failure, but `{shown}` cannot fail"),
          )
          .with_help("drop the `!`")
          .with_note("only a call to a function declared `: T!` can fail"),
        );
      }
      return resolved;
    };
    if !self.signature.failable {
      let shown = self.show(self.signature.ret);
      self.report(
        CompileError::at(
          ErrorCode::PropagateErrorInNonFailable,
          span,
          "`!` propagates the failure out of a function that cannot fail",
        )
        .with_help(format!("declare the return type as `{shown}!`"))
        .with_note("or handle it here with `catch`"),
      );
    }
    inner
  }

  fn check_catch(&mut self, operand: &Expr, handler: &CatchHandler, span: Span) -> TypeId {
    let ty = self.check_expr(operand, None);
    let resolved = self.context.shallow_resolve(ty);
    let inner = match *self.context.kind(resolved) {
      TypeKind::Failable(inner) => inner,
      TypeKind::Error | TypeKind::Never => TypeId::ERROR,
      _ => {
        let shown = self.show(resolved);
        let already_consumed = matches!(operand.kind, ExprKind::ErrorPropagate(_));
        let error = if already_consumed {
          CompileError::at(
            ErrorCode::CatchAfterPropagate,
            span,
            "`!` has already propagated the failure, so there is nothing left to catch",
          )
          .with_help("drop the `!`, or drop the `catch`")
        } else {
          CompileError::at(
            ErrorCode::CatchOnNonFailable,
            span,
            format!("`{shown}` cannot fail, so `catch` would never run"),
          )
          .with_note("only a call to a function declared `: T!` can fail")
        };
        self.report(error);
        resolved
      }
    };

    match handler {
      CatchHandler::Discard(block) => {
        let diverges = self.check_block(block);
        self.demand_diverging_handler(diverges, block.span);
      }
      CatchHandler::Bind { name, block } => {
        let error_type = self.prelude.error_type;
        self.bind_local(name, error_type);
        let diverges = self.check_block(block);
        self.demand_diverging_handler(diverges, block.span);
      }
      CatchHandler::Value(value) => {
        let fallback = self.check_value(value, Some(inner));
        self.expect_assignable(inner, fallback, value.span, "this `catch` fallback");
      }
    }
    inner
  }

  fn demand_diverging_handler(&mut self, diverges: bool, span: Span) {
    if diverges {
      return;
    }
    self.report(
      CompileError::at(
        ErrorCode::CatchBlockFallsThrough,
        span,
        "every path through a `catch` block must leave the enclosing code",
      )
      .with_caret("this block can finish normally")
      .with_note(
        "Pump has no value-producing block, so a handler that falls through \
            would leave the binding with no value",
      )
      .with_help(
        "`return`, `fail`, `break`, `continue`, or use the value form: \
            `expr catch <value>`",
      ),
    );
  }

  fn demand_inferred(&mut self, ty: TypeId, span: Span, what: &str) -> bool {
    let resolved = self.context.resolve(ty);
    if !self.has_inference_variable(resolved) {
      return true;
    }
    self.report(
      CompileError::at(
        ErrorCode::CannotInferType,
        span,
        format!("the type arguments of {what} cannot be inferred here"),
      )
      .with_help("annotate the binding, or write the type arguments: `::<int>`"),
    );
    false
  }

  fn has_inference_variable(&self, ty: TypeId) -> bool {
    let ty = self.context.shallow_resolve(ty);
    match self.context.kind(ty) {
      TypeKind::Var(_) => true,
      TypeKind::Array(inner)
      | TypeKind::Set(inner)
      | TypeKind::Optional(inner)
      | TypeKind::Failable(inner) => self.has_inference_variable(*inner),
      TypeKind::Map { key, value } => {
        self.has_inference_variable(*key) || self.has_inference_variable(*value)
      }
      TypeKind::Tuple(parts) => parts.iter().any(|&part| self.has_inference_variable(part)),
      TypeKind::Function(signature) => {
        signature
          .params
          .iter()
          .any(|&param| self.has_inference_variable(param))
          || signature
            .variadic
            .is_some_and(|element| self.has_inference_variable(element))
          || self.has_inference_variable(signature.ret)
      }
      TypeKind::Named { args, .. } => {
        args.iter().any(|&arg| self.has_inference_variable(arg))
      }
      _ => false,
    }
  }
}

struct DirectCall {
  func: FuncId,
  callee: Callee,
  receiver: Option<NodeId>,
  owner_args: Vec<TypeId>,
  explicit: Option<(Vec<TypeId>, Span)>,
}

enum Slot<'a> {
  One(&'a Expr),
  Many(Vec<&'a Expr>),
}

impl Checker<'_> {
  fn check_call(
    &mut self,
    expr: &Expr,
    callee: &Expr,
    args: &[Argument],
    expected: Option<TypeId>,
  ) -> TypeId {
    // turbofish thuoc ve loi goi chu khong thuoc ve cai bi goi:
    // `f::<int>(x)` parse thanh mot loi goi ma callee la `f::<int>` (14.17)
    let (base, explicit) = match &callee.kind {
      ExprKind::TypeArgs {
        base,
        args: written,
      } => {
        let types: Vec<TypeId> = written
          .iter()
          .map(|argument| {
            self.written_types
              .get(&argument.id)
              .copied()
              .unwrap_or(TypeId::ERROR)
          })
          .collect();
        (base.as_ref(), Some((types, callee.span)))
      }
      _ => (callee, None),
    };

    match &base.kind {
      ExprKind::Ident(name) => {
        self.check_named_call(expr, base, name, explicit, args, expected)
      }
      ExprKind::Field {
        base: receiver,
        name,
      } => self.check_member_call(expr, base, receiver, name, explicit, args, expected),
      ExprKind::This => {
        self.report(
          CompileError::at(
            ErrorCode::NotCallable,
            base.span,
            "`this` is the receiver, not a function",
          )
          .with_help("call one of its methods: `this.f()`"),
        );
        self.check_arguments_for_recovery(args);
        TypeId::ERROR
      }
      _ => {
        let ty = self.check_value(base, None);
        self.reject_turbofish(&explicit);
        self.check_indirect_call(expr.id, ty, args, expr.span)
      }
    }
  }

  fn check_named_call(
    &mut self,
    expr: &Expr,
    base: &Expr,
    name: &Ident,
    explicit: Option<(Vec<TypeId>, Span)>,
    args: &[Argument],
    expected: Option<TypeId>,
  ) -> TypeId {
    let Some(binding) = self.values.get(&base.id).copied() else {
      self.check_arguments_for_recovery(args);
      return TypeId::ERROR;
    };
    match binding {
      ValueBinding::Function(func) => self.check_declared_call(
        expr.id,
        DirectCall {
          func,
          callee: Callee::Function(func),
          receiver: None,
          owner_args: Vec::new(),
          explicit,
        },
        args,
        expr.span,
        expected,
      ),
      ValueBinding::Method(func) => {
        let Some(owner) = self.context.func(func).owner else {
          self.check_arguments_for_recovery(args);
          return TypeId::ERROR;
        };
        let receiver_type = self.signature.this.unwrap_or(TypeId::ERROR);
        let owner_args = self.owner_arguments(receiver_type);
        self.check_declared_call(
          expr.id,
          DirectCall {
            func,
            callee: Callee::Method { owner, func },
            // goi method khong ghi ro receiver o trong than mot
            // method nghia la `this.f()` (D-23), buoc lower se dat
            // receiver vao
            receiver: Some(NodeId::NONE),
            owner_args,
            explicit,
          },
          args,
          expr.span,
          expected,
        )
      }
      ValueBinding::Predeclared(value) => {
        self.reject_turbofish(&explicit);
        self.check_predeclared_call(expr.id, value, args, expr.span)
      }
      ValueBinding::Conversion(target) => {
        self.reject_turbofish(&explicit);
        self.check_conversion_call(expr.id, target, args, expr.span)
      }
      ValueBinding::Local(_)
      | ValueBinding::Captured(_)
      | ValueBinding::Field { .. }
      | ValueBinding::GlobalConst(_) => {
        let ty = self.check_value(base, None);
        self.reject_turbofish(&explicit);
        self.check_indirect_call(expr.id, ty, args, expr.span)
      }
      ValueBinding::Type(def) => {
        let what = self.context.def(def).name.clone();
        self.report(
          CompileError::at(
            ErrorCode::NotCallable,
            name.span,
            format!("`{what}` is a type, not a function"),
          )
          .with_help(format!("build a value with `{what} {{ ... }}`")),
        );
        self.check_arguments_for_recovery(args);
        TypeId::ERROR
      }
      ValueBinding::Module(_) => {
        self.report(
          CompileError::at(
            ErrorCode::NotCallable,
            name.span,
            format!("`{}` is a module, not a function", name.name),
          )
          .with_help(format!("call something inside it: `{}.f()`", name.name)),
        );
        self.check_arguments_for_recovery(args);
        TypeId::ERROR
      }
    }
  }

  fn check_member_call(
    &mut self,
    expr: &Expr,
    callee: &Expr,
    receiver: &Expr,
    name: &Ident,
    explicit: Option<(Vec<TypeId>, Span)>,
    args: &[Argument],
    expected: Option<TypeId>,
  ) -> TypeId {
    // `module.f(...)`: resolver da buoc ca duong dan roi
    if let Some(binding) = self.values.get(&callee.id).copied() {
      return match binding {
        ValueBinding::Function(func) => self.check_declared_call(
          expr.id,
          DirectCall {
            func,
            callee: Callee::Function(func),
            receiver: None,
            owner_args: Vec::new(),
            explicit,
          },
          args,
          expr.span,
          expected,
        ),
        ValueBinding::GlobalConst(global) => {
          let ty = self.global_types[global.index()];
          self.record(callee.id, ty);
          self.reject_turbofish(&explicit);
          self.check_indirect_call(expr.id, ty, args, expr.span)
        }
        other => {
          let ty = self.module_member_type(other, name);
          self.record(callee.id, ty);
          self.check_arguments_for_recovery(args);
          TypeId::ERROR
        }
      };
    }

    // `Enum.Variant(...)`: goc la ten mot kieu (16.7)
    if let Some(def) = self.type_path_base(receiver) {
      self.reject_turbofish(&explicit);
      return self.check_variant_call(expr.id, def, name, args, expr.span, expected);
    }

    let receiver_type = self.check_value(receiver, None);
    match self.member_of(receiver, receiver_type, name) {
      Member::Method {
        owner,
        func,
        receiver: applied,
      } => {
        let owner_args = self.owner_arguments(applied);
        self.check_declared_call(
          expr.id,
          DirectCall {
            func,
            callee: Callee::Method { owner, func },
            receiver: Some(receiver.id),
            owner_args,
            explicit,
          },
          args,
          expr.span,
          expected,
        )
      }
      Member::InterfaceMethod {
        interface,
        slot,
        func,
      } => {
        let owner_args = self.owner_arguments(receiver_type);
        self.check_declared_call(
          expr.id,
          DirectCall {
            func,
            callee: Callee::Interface { interface, slot },
            receiver: Some(receiver.id),
            owner_args,
            explicit,
          },
          args,
          expr.span,
          expected,
        )
      }
      Member::Builtin {
        method,
        params,
        ret,
      } => {
        self.reject_turbofish(&explicit);
        self.check_builtin_call(expr.id, method, receiver.id, params, ret, args, expr.span)
      }
      Member::Length => {
        self.field_accesses.insert(callee.id, FieldAccess::Length);
        self.record(callee.id, TypeId::INT);
        self.report(
          CompileError::at(
            ErrorCode::NotCallable,
            expr.span,
            "`length` is a field, not a method",
          )
          .with_help("drop the parentheses"),
        );
        self.check_arguments_for_recovery(args);
        TypeId::INT
      }
      other => {
        // mot truong kieu ham, hoac cai gi do da bao loi roi
        let ty = self.member_value_type(callee.id, other, name);
        self.record(callee.id, ty);
        self.reject_turbofish(&explicit);
        self.check_indirect_call(expr.id, ty, args, expr.span)
      }
    }
  }

  fn check_declared_call(
    &mut self,
    node: NodeId,
    target: DirectCall,
    args: &[Argument],
    span: Span,
    expected: Option<TypeId>,
  ) -> TypeId {
    let DirectCall {
      func,
      callee,
      receiver,
      owner_args,
      explicit,
    } = target;
    let definition = self.context.func(func).clone();
    let arity = definition.generics.len();
    let mut sound = true;

    let func_args: Vec<TypeId> = match &explicit {
      Some((written, at)) if written.len() == arity => written.clone(),
      Some((written, at)) => {
        self.report(
          CompileError::at(
            ErrorCode::WrongTypeArgumentCount,
            *at,
            format!(
              "`{}` takes {arity} type argument{}, but {} {} written",
              definition.name,
              if arity == 1 { "" } else { "s" },
              written.len(),
              if written.len() == 1 { "was" } else { "were" }
            ),
          )
          .with_secondary(definition.span, "declared here"),
        );
        sound = false;
        (0..arity).map(|_| self.context.fresh_var()).collect()
      }
      None => (0..arity).map(|_| self.context.fresh_var()).collect(),
    };

    let mut slots: Vec<Option<Slot>> = (0..definition.params.len()).map(|_| None).collect();
    let named_from = args
      .iter()
      .position(|argument| argument.name.is_some())
      .unwrap_or(args.len());
    let (positional, named) = args.split_at(named_from);

    let mut consumed = 0usize;
    for (index, param) in definition.params.iter().enumerate() {
      if consumed >= positional.len() {
        break;
      }
      if param.variadic {
        slots[index] = Some(Slot::Many(
          positional[consumed..]
            .iter()
            .map(|argument| &argument.value)
            .collect(),
        ));
        consumed = positional.len();
        break;
      }
      slots[index] = Some(Slot::One(&positional[consumed].value));
      consumed += 1;
    }
    if consumed < positional.len() {
      self.report(
        CompileError::at(
          ErrorCode::WrongArgumentCount,
          positional[consumed].value.span,
          format!(
            "`{}` takes {} argument{}, but {} were supplied",
            definition.name,
            definition.params.len(),
            if definition.params.len() == 1 {
              ""
            } else {
              "s"
            },
            positional.len()
          ),
        )
        .with_secondary(definition.span, "declared here"),
      );
      sound = false;
      for extra in &positional[consumed..] {
        self.check_value(&extra.value, None);
      }
    }

    for argument in named {
      let Some(label) = &argument.name else {
        // doi so vi tri dung sau doi so co ten, parser bao roi
        self.check_value(&argument.value, None);
        sound = false;
        continue;
      };
      let Some(index) = definition.param_index(&label.name) else {
        let names: Vec<&str> = definition
          .params
          .iter()
          .map(|param| param.name.as_str())
          .collect();
        self.report(
          CompileError::at(
            ErrorCode::UnknownNamedArgument,
            label.span,
            format!(
              "`{}` has no parameter named `{}`",
              definition.name, label.name
            ),
          )
          .with_secondary(definition.span, "declared here")
          .with_note(format!("its parameters are {}", join_names(&names))),
        );
        self.check_value(&argument.value, None);
        sound = false;
        continue;
      };
      if definition.params[index].variadic {
        self.report(
          CompileError::at(
            ErrorCode::VariadicPassedByName,
            label.span,
            format!(
              "`{}` is variadic, so it collects positional arguments",
              label.name
            ),
          )
          .with_help("pass its values positionally"),
        );
        self.check_value(&argument.value, None);
        sound = false;
        continue;
      }
      if slots[index].is_some() {
        self.report(
          CompileError::at(
            ErrorCode::ArgumentSuppliedTwice,
            label.span,
            format!("`{}` was already filled positionally", label.name),
          )
          .with_help("drop the positional argument, or drop the name"),
        );
        self.check_value(&argument.value, None);
        sound = false;
        continue;
      }
      slots[index] = Some(Slot::One(&argument.value));
    }

    let mut bound: Vec<BoundArgument> = Vec::new();
    if definition.has_receiver {
      bound.push(BoundArgument::Receiver(receiver.unwrap_or(NodeId::NONE)));
    }
    for (index, param) in definition.params.iter().enumerate() {
      let param_type =
        self.instantiate(param.ty, definition.owner, &owner_args, func, &func_args);
      match slots[index].take() {
        Some(Slot::One(value)) => {
          let ty = self.check_value(value, Some(param_type));
          let position = format!("the parameter `{}`", param.name);
          self.expect_assignable(param_type, ty, value.span, &position);
          bound.push(BoundArgument::Expression(value.id));
        }
        Some(Slot::Many(values)) => {
          let mut ids = Vec::with_capacity(values.len());
          for value in values {
            let ty = self.check_value(value, Some(param_type));
            let position = format!("the variadic parameter `{}`", param.name);
            self.expect_assignable(param_type, ty, value.span, &position);
            ids.push(value.id);
          }
          bound.push(BoundArgument::Variadic(ids));
        }
        None if param.variadic => bound.push(BoundArgument::Variadic(Vec::new())),
        None => match param.default.clone() {
          Some(default) => bound.push(BoundArgument::Default(default)),
          None => {
            let shown = self.show(param_type);
            self.report(
              CompileError::at(
                ErrorCode::MissingArgument,
                span,
                format!(
                  "`{}` needs `{}: {shown}`, which was not supplied",
                  definition.name, param.name
                ),
              )
              .with_secondary(param.span, "declared here")
              .with_help(format!("pass it by name: `{}: <{shown}>`", param.name)),
            );
            sound = false;
          }
        },
      }
    }

    // cai ky vong la thu keo suy dien cho kieu tra ve ma doi so khong he
    // nhac toi, vi du `fn empty<T>(): [T]`
    let ret = self.instantiate(
      definition.ret,
      definition.owner,
      &owner_args,
      func,
      &func_args,
    );
    if let Some(expectation) = expected {
      if !definition.failable {
        self.unify(expectation, ret);
      }
    }

    if let Some(owner) = definition.owner {
      if !self.check_generic_bounds(GenericOwner::Type(owner), &owner_args, span) {
        sound = false;
      }
    }
    if !self.check_generic_bounds(GenericOwner::Func(func), &func_args, span) {
      sound = false;
    }

    let mut type_arguments: Vec<TypeId> = owner_args;
    type_arguments.extend(func_args);
    let type_arguments: Vec<TypeId> = type_arguments
      .into_iter()
      .map(|ty| self.context.resolve(ty))
      .collect();
    if type_arguments
      .iter()
      .any(|&argument| self.has_inference_variable(argument))
    {
      let name = definition.name.clone();
      self.report(
        CompileError::at(
          ErrorCode::CannotInferType,
          span,
          format!("the type arguments of `{name}` cannot be inferred from this call"),
        )
        .with_secondary(definition.span, "declared here")
        .with_help(format!("write them out: `{name}::<int>(...)`")),
      );
      sound = false;
    }

    if sound {
      self.calls.insert(
        node,
        ResolvedCall {
          callee,
          arguments: bound,
          type_arguments: type_arguments.clone(),
          failable: definition.failable,
        },
      );
      // chu ky method cua interface khong co than de mono, ban cu the
      // di vao qua bang conformance
      if definition.has_body {
        let instantiation = Instantiation {
          func,
          type_arguments,
        };
        if self.seen_instantiations.insert(instantiation.clone()) {
          self.instantiations.push(instantiation);
        }
      }
    }

    let ret = self.context.resolve(ret);
    if definition.failable {
      self.context.failable_of(ret)
    } else {
      ret
    }
  }

  fn check_indirect_call(
    &mut self,
    node: NodeId,
    callee: TypeId,
    args: &[Argument],
    span: Span,
  ) -> TypeId {
    let resolved = self.context.shallow_resolve(callee);
    let TypeKind::Function(signature) = self.context.kind(resolved).clone() else {
      if !matches!(
        self.context.kind(resolved),
        TypeKind::Error | TypeKind::Never
      ) {
        let shown = self.show(resolved);
        self.report(
          CompileError::at(
            ErrorCode::NotCallable,
            span,
            format!("`{shown}` is not a function"),
          )
          .with_caret(format!("this is `{shown}`")),
        );
      }
      self.check_arguments_for_recovery(args);
      return TypeId::ERROR;
    };

    let mut sound = true;
    for argument in args {
      if let Some(label) = &argument.name {
        self.report(
          CompileError::at(
            ErrorCode::NamedArgumentThroughValue,
            label.span,
            "a function type carries no parameter names",
          )
          .with_note("named arguments need a direct call to a declared function")
          .with_help("pass this argument positionally"),
        );
        sound = false;
      }
    }

    let supplied: Vec<&Expr> = args.iter().map(|argument| &argument.value).collect();
    let fixed = signature.params.len();
    let too_few = supplied.len() < fixed;
    let too_many = signature.variadic.is_none() && supplied.len() > fixed;
    if too_few || too_many {
      self.report(CompileError::at(
        ErrorCode::WrongArgumentCount,
        span,
        format!(
          "this closure takes {fixed}{} argument{}, but {} {} supplied",
          if signature.variadic.is_some() {
            " or more"
          } else {
            ""
          },
          if fixed == 1 { "" } else { "s" },
          supplied.len(),
          if supplied.len() == 1 { "was" } else { "were" }
        ),
      ));
      sound = false;
    }

    let mut bound = Vec::new();
    for (index, &param) in signature.params.iter().enumerate() {
      let Some(&value) = supplied.get(index) else {
        break;
      };
      let ty = self.check_value(value, Some(param));
      self.expect_assignable(param, ty, value.span, "this argument");
      bound.push(BoundArgument::Expression(value.id));
    }
    match signature.variadic {
      Some(element) => {
        let mut ids = Vec::new();
        for &value in supplied.iter().skip(fixed) {
          let ty = self.check_value(value, Some(element));
          self.expect_assignable(element, ty, value.span, "this variadic argument");
          ids.push(value.id);
        }
        bound.push(BoundArgument::Variadic(ids));
      }
      None => {
        for &value in supplied.iter().skip(fixed) {
          self.check_value(value, None);
        }
      }
    }

    if sound {
      self.calls.insert(
        node,
        ResolvedCall {
          callee: Callee::Closure,
          arguments: bound,
          type_arguments: Vec::new(),
          failable: signature.failable,
        },
      );
    }
    if signature.failable {
      self.context.failable_of(signature.ret)
    } else {
      signature.ret
    }
  }

  fn check_builtin_call(
    &mut self,
    node: NodeId,
    method: BuiltinMethod,
    receiver: NodeId,
    params: Vec<TypeId>,
    ret: TypeId,
    args: &[Argument],
    span: Span,
  ) -> TypeId {
    let mut sound = self.demand_positional(args, "a builtin method");
    if args.len() != params.len() {
      self.report(CompileError::at(
        ErrorCode::WrongArgumentCount,
        span,
        format!(
          "`{}` takes {} argument{}, but {} {} supplied",
          method.spelling(),
          params.len(),
          if params.len() == 1 { "" } else { "s" },
          args.len(),
          if args.len() == 1 { "was" } else { "were" }
        ),
      ));
      sound = false;
    }

    let mut bound = vec![BoundArgument::Receiver(receiver)];
    for (index, argument) in args.iter().enumerate() {
      match params.get(index) {
        Some(&param) => {
          let ty = self.check_value(&argument.value, Some(param));
          self.expect_assignable(param, ty, argument.value.span, "this argument");
          bound.push(BoundArgument::Expression(argument.value.id));
        }
        None => {
          self.check_value(&argument.value, None);
        }
      }
    }

    if sound {
      self.calls.insert(
        node,
        ResolvedCall {
          callee: Callee::Builtin(method),
          arguments: bound,
          type_arguments: Vec::new(),
          failable: false,
        },
      );
    }
    ret
  }

  fn check_predeclared_call(
    &mut self,
    node: NodeId,
    value: Predeclared,
    args: &[Argument],
    span: Span,
  ) -> TypeId {
    let mut sound = self.demand_positional(args, "a builtin");
    let (least, most) = match value {
      Predeclared::Assert => (1, 2),
      Predeclared::OsArgs | Predeclared::OsError => (0, 0),
      Predeclared::WriteFileText | Predeclared::WriteFileBytes | Predeclared::OsRun => (2, 2),
      _ => (1, 1),
    };
    if args.len() < least || args.len() > most {
      let wanted = if least == most {
        format!("{least} argument{}", if least == 1 { "" } else { "s" })
      } else {
        format!("{least} or {most} arguments")
      };
      self.report(CompileError::at(
        ErrorCode::WrongArgumentCount,
        span,
        format!(
          "`{}` takes {wanted}, but {} {} supplied",
          value.spelling(),
          args.len(),
          if args.len() == 1 { "was" } else { "were" }
        ),
      ));
      sound = false;
    }

    let mut bound = Vec::new();
    match value {
      Predeclared::Print | Predeclared::Println => {
        if let Some(argument) = args.first() {
          let ty = self.check_value(&argument.value, None);
          self.check_interpolation(ty, argument.value.span);
          bound.push(BoundArgument::Expression(argument.value.id));
        }
      }
      Predeclared::Panic => {
        if let Some(argument) = args.first() {
          let ty = self.check_value(&argument.value, Some(TypeId::STRING));
          self.expect_assignable(
            TypeId::STRING,
            ty,
            argument.value.span,
            "the panic message",
          );
          bound.push(BoundArgument::Expression(argument.value.id));
        }
      }
      Predeclared::Assert => {
        if let Some(argument) = args.first() {
          let ty = self.check_value(&argument.value, Some(TypeId::BOOL));
          self.demand_bool(ty, argument.value.span);
          bound.push(BoundArgument::Expression(argument.value.id));
        }
        if let Some(argument) = args.get(1) {
          let ty = self.check_value(&argument.value, Some(TypeId::STRING));
          self.expect_assignable(
            TypeId::STRING,
            ty,
            argument.value.span,
            "the assertion message",
          );
          bound.push(BoundArgument::Expression(argument.value.id));
        }
      }
      Predeclared::Len => {
        if let Some(argument) = args.first() {
          let ty = self.check_value(&argument.value, None);
          let resolved = self.context.shallow_resolve(ty);
          if !self.has_length(resolved)
            && !matches!(
              self.context.kind(resolved),
              TypeKind::Error | TypeKind::Never
            )
          {
            let shown = self.show(resolved);
            self.report(
              CompileError::at(
                ErrorCode::TypeMismatch,
                argument.value.span,
                format!("`len` has no meaning for `{shown}`"),
              )
              .with_note("`len` takes an array, a map, a set or a string"),
            );
          }
          bound.push(BoundArgument::Expression(argument.value.id));
        }
      }
      // May cua vao he dieu hanh. Tham so dau bao gio cung la mot duong
      // dan hoac mot ten chuong trinh, nen kiem chung mot cho.
      Predeclared::ReadFileText
      | Predeclared::ReadFileBytes
      | Predeclared::WriteFileText
      | Predeclared::WriteFileBytes
      | Predeclared::OsRun => {
        if let Some(argument) = args.first() {
          let ty = self.check_value(&argument.value, Some(TypeId::STRING));
          self.expect_assignable(TypeId::STRING, ty, argument.value.span, "the path");
          bound.push(BoundArgument::Expression(argument.value.id));
        }
        if let Some(argument) = args.get(1) {
          let wanted = self.predeclared_payload(value);
          let ty = self.check_value(&argument.value, Some(wanted));
          self.expect_assignable(wanted, ty, argument.value.span, "the data");
          bound.push(BoundArgument::Expression(argument.value.id));
        }
      }
      Predeclared::OsArgs | Predeclared::OsError => {}
    }
    for extra in args.iter().skip(most) {
      self.check_value(&extra.value, None);
    }

    if sound {
      self.calls.insert(
        node,
        ResolvedCall {
          callee: Callee::Predeclared(value),
          arguments: bound,
          type_arguments: Vec::new(),
          failable: false,
        },
      );
    }
    match value {
      Predeclared::Panic => TypeId::NEVER,
      Predeclared::Len => TypeId::INT,
      // Doc thi tra ve `T?`, null la hong. Ghi thi tra ve co/khong.
      // Cai gi hong thi `os_error()` noi. Xem `runtime/src/os.rs`.
      Predeclared::ReadFileText => self.context.optional_of(TypeId::STRING),
      Predeclared::ReadFileBytes => {
        let bytes = self.context.array_of(TypeId::INT);
        self.context.optional_of(bytes)
      }
      Predeclared::WriteFileText | Predeclared::WriteFileBytes => TypeId::BOOL,
      Predeclared::OsArgs => self.context.array_of(TypeId::STRING),
      Predeclared::OsRun => self.context.optional_of(TypeId::INT),
      Predeclared::OsError => TypeId::STRING,
      _ => TypeId::VOID,
    }
  }

  /// The type of the second argument of a two-argument builtin.
  fn predeclared_payload(&mut self, value: Predeclared) -> TypeId {
    match value {
      Predeclared::WriteFileText => TypeId::STRING,
      Predeclared::WriteFileBytes => self.context.array_of(TypeId::INT),
      Predeclared::OsRun => self.context.array_of(TypeId::STRING),
      _ => TypeId::ERROR,
    }
  }

  fn check_conversion_call( &mut self, node: NodeId, target: TypeId, args: &[Argument], span: Span, ) -> TypeId {
    let mut sound = self.demand_positional(args, "a conversion");
    let shown_target = self.show(target);
    if args.len() != 1 {
      self.report(
        CompileError::at(
          ErrorCode::WrongArgumentCount,
          span,
          format!(
            "`{shown_target}` converts exactly one value, but {} {} supplied",
            args.len(),
            if args.len() == 1 { "was" } else { "were" }
          ),
        )
        .with_help(format!("write `{shown_target}(x)`")),
      );
      self.check_arguments_for_recovery(args);
      return target;
    }

    let source = self.check_value(&args[0].value, None);
    let source = self.context.shallow_resolve(source);
    if !self.conversion_is_legal(target, source) {
      let shown_source = self.show(source);
      self.report(
        CompileError::at(
          ErrorCode::InvalidConversion,
          args[0].value.span,
          format!("`{shown_source}` cannot be converted to `{shown_target}`"),
        )
        .with_note(
          "the conversions are `int`, `uint` and `float` between the numbers and \
           `char`, `char` from an integer, and `string` from any primitive",
        ),
      );
      sound = false;
    }

    if sound {
      self.calls.insert(
        node,
        ResolvedCall {
          callee: Callee::Conversion { target },
          arguments: vec![BoundArgument::Expression(args[0].value.id)],
          type_arguments: Vec::new(),
          failable: false,
        },
      );
    }
    target
  }

  fn conversion_is_legal(&mut self, target: TypeId, source: TypeId) -> bool {
    if matches!(self.context.kind(source), TypeKind::Error | TypeKind::Never) {
      return true;
    }
    let is_char = matches!(self.context.kind(source), TypeKind::Char);
    match target {
      TypeId::INT | TypeId::UINT => self.context.is_numeric(source) || is_char,
      TypeId::FLOAT => self.context.is_numeric(source),
      TypeId::CHAR => self.context.is_integer(source) || is_char,
      TypeId::STRING => {
        let primitive = matches!(
          self.context.kind(source),
          TypeKind::Bool
            | TypeKind::Int
            | TypeKind::Uint
            | TypeKind::Float
            | TypeKind::Char
            | TypeKind::String
            | TypeKind::UntypedInt
            | TypeKind::UntypedFloat
        );
        let stringable = self.prelude.stringable;
        primitive || self.record_conformance(stringable, source).is_some()
      }
      _ => false,
    }
  }

  fn check_variant_call(
    &mut self,
    node: NodeId,
    def: DefId,
    name: &Ident,
    args: &[Argument],
    span: Span,
    expected: Option<TypeId>,
  ) -> TypeId {
    let owner = self.context.def(def).name.clone();
    let Some(enumeration) = self.context.def(def).as_enum().cloned() else {
      self.report(
        CompileError::at(
          ErrorCode::NotAnEnum,
          name.span,
          format!(
            "`{owner}` is not an enum, so `{owner}.{}` names nothing",
            name.name
          ),
        )
        .with_help(format!("build a struct with `{owner} {{ ... }}`")),
      );
      self.check_arguments_for_recovery(args);
      return TypeId::ERROR;
    };
    let Some(index) = enumeration.variant_index(&name.name) else {
      let names: Vec<&str> = enumeration
        .variants
        .iter()
        .map(|variant| variant.name.as_str())
        .collect();
      self.report(
        CompileError::at(
          ErrorCode::UnknownVariant,
          name.span,
          format!("`{owner}` has no variant `{}`", name.name),
        )
        .with_note(format!("its variants are {}", join_names(&names))),
      );
      self.check_arguments_for_recovery(args);
      return TypeId::ERROR;
    };

    let arguments = self.enum_arguments(def, expected);
    let payload: Vec<TypeId> = enumeration.variants[index]
      .payload
      .clone()
      .into_iter()
      .map(|ty| {
        self.context
          .substitute(ty, GenericOwner::Type(def), &arguments)
      })
      .collect();

    let mut sound = self.demand_positional(args, "a variant payload");
    if args.len() != payload.len() {
      self.report(
        CompileError::at(
          ErrorCode::WrongArgumentCount,
          span,
          format!(
            "`{owner}.{}` carries {} value{}, but {} {} supplied",
            name.name,
            payload.len(),
            if payload.len() == 1 { "" } else { "s" },
            args.len(),
            if args.len() == 1 { "was" } else { "were" }
          ),
        )
        .with_secondary(enumeration.variants[index].span, "declared here"),
      );
      sound = false;
    }

    let mut bound = Vec::new();
    for (position, argument) in args.iter().enumerate() {
      match payload.get(position) {
        Some(&ty) => {
          let actual = self.check_value(&argument.value, Some(ty));
          let context = format!("the payload of `{owner}.{}`", name.name);
          self.expect_assignable(ty, actual, argument.value.span, &context);
          bound.push(BoundArgument::Expression(argument.value.id));
        }
        None => {
          self.check_value(&argument.value, None);
        }
      }
    }

    let ty = self.context.named(def, arguments);
    let what = format!("`{owner}.{}`", name.name);
    if !self.demand_inferred(ty, span, &what) {
      sound = false;
    }
    let ty = self.context.resolve(ty);
    if sound {
      let type_arguments = match self.context.kind(ty).clone() {
        TypeKind::Named { args, .. } => args,
        _ => Vec::new(),
      };
      self.calls.insert(
        node,
        ResolvedCall {
          callee: Callee::Variant {
            def,
            variant: index as u32,
          },
          arguments: bound,
          type_arguments,
          failable: false,
        },
      );
    }
    ty
  }

  fn instantiate(
    &mut self,
    ty: TypeId,
    owner: Option<DefId>,
    owner_args: &[TypeId],
    func: FuncId,
    func_args: &[TypeId],
  ) -> TypeId {
    let mut ty = ty;
    if let Some(owner) = owner {
      if !owner_args.is_empty() {
        ty = self
          .context
          .substitute(ty, GenericOwner::Type(owner), owner_args);
      }
    }
    if !func_args.is_empty() {
      ty = self
        .context
        .substitute(ty, GenericOwner::Func(func), func_args);
    }
    ty
  }

  fn owner_arguments(&self, receiver: TypeId) -> Vec<TypeId> {
    match self.context.kind(self.context.shallow_resolve(receiver)) {
      TypeKind::Named { args, .. } => args.clone(),
      _ => Vec::new(),
    }
  }

  fn demand_positional(&mut self, args: &[Argument], what: &str) -> bool {
    let mut positional = true;
    for argument in args {
      if let Some(label) = &argument.name {
        self.report(
          CompileError::at(
            ErrorCode::NamedArgumentThroughValue,
            label.span,
            format!("{what} has no parameter names"),
          )
          .with_help("pass this argument positionally"),
        );
        positional = false;
      }
    }
    positional
  }

  fn reject_turbofish(&mut self, explicit: &Option<(Vec<TypeId>, Span)>) {
    let Some((_, span)) = explicit else { return };
    self.report(
      CompileError::at(
        ErrorCode::TurbofishNotAllowed,
        *span,
        "this callee takes no type arguments",
      )
      .with_help("drop the `::<...>`"),
    );
  }

  fn check_arguments_for_recovery(&mut self, args: &[Argument]) {
    for argument in args {
      self.check_value(&argument.value, None);
    }
  }
}

impl Checker<'_> {
  fn check_struct_literal(&mut self, expr: &Expr, expected: Option<TypeId>) -> TypeId {
    let ExprKind::StructLit(literal) = &expr.kind else {
      return TypeId::ERROR;
    };
    let Some(ValueBinding::Type(def)) = self.values.get(&expr.id).copied() else {
      for field in &literal.fields {
        self.check_value(&field.value, None);
      }
      return TypeId::ERROR;
    };

    let name = self.context.def(def).name.clone();
    let Some(structure) = self.context.def(def).as_struct().cloned() else {
      self.report(
        CompileError::at(
          ErrorCode::NotAStruct,
          literal.path.span,
          format!("`{name}` is not a struct, so it has no field initialisers"),
        )
        .with_help("name an enum variant with `Enum.Variant`"),
      );
      for field in &literal.fields {
        self.check_value(&field.value, None);
      }
      return TypeId::ERROR;
    };

    let arguments = self.struct_arguments(def, literal, expected);

    let mut initialised = vec![false; structure.fields.len()];
    for field in &literal.fields {
      let Some(index) = structure.field_index(&field.name.name) else {
        let names: Vec<&str> = structure
          .fields
          .iter()
          .map(|declared| declared.name.as_str())
          .collect();
        self.report(
          CompileError::at(
            ErrorCode::UnknownStructField,
            field.name.span,
            format!("`{name}` has no field `{}`", field.name.name),
          )
          .with_note(format!("its fields are {}", join_names(&names))),
        );
        self.check_value(&field.value, None);
        continue;
      };
      if initialised[index] {
        self.report(
          CompileError::at(
            ErrorCode::DuplicateStructFieldInit,
            field.name.span,
            format!("`{}` is initialised twice", field.name.name),
          )
          .with_help("remove one of the two"),
        );
      }
      initialised[index] = true;

      let declared = structure.fields[index].clone();
      self.check_field_visibility(def, &declared, field.name.span);
      let ty = self
        .context
        .substitute(declared.ty, GenericOwner::Type(def), &arguments);
      let value = self.check_value(&field.value, Some(ty));
      let position = format!("the field `{}`", field.name.name);
      self.expect_assignable(ty, value, field.value.span, &position);
    }

    let missing: Vec<&str> = structure
      .fields
      .iter()
      .zip(&initialised)
      .filter(|(_, seen)| !**seen)
      .map(|(field, _)| field.name.as_str())
      .collect();
    if !missing.is_empty() {
      self.report(
        CompileError::at(
          ErrorCode::MissingStructField,
          literal.span,
          format!("this `{name}` is missing {}", join_names(&missing)),
        )
        .with_secondary(self.context.def(def).span, "declared here")
        .with_note("Pump 1.0 has no field defaults, so every field must be given"),
      );
    }

    let ty = self.context.named(def, arguments);
    self.demand_inferred(ty, literal.path.span, &format!("`{name}`"));
    self.context.resolve(ty)
  }

  fn struct_arguments(
    &mut self,
    def: DefId,
    literal: &crate::ast::StructLit,
    expected: Option<TypeId>,
  ) -> Vec<TypeId> {
    let arity = self.context.def(def).generics.len();
    if !literal.type_args.is_empty() {
      let written: Vec<TypeId> = literal
        .type_args
        .iter()
        .map(|argument| {
          self.written_types
            .get(&argument.id)
            .copied()
            .unwrap_or(TypeId::ERROR)
        })
        .collect();
      if written.len() == arity {
        return written;
      }
      let name = self.context.def(def).name.clone();
      let declared = self.context.def(def).span;
      self.report(
        CompileError::at(
          ErrorCode::WrongTypeArgumentCount,
          literal.path.span,
          format!(
            "`{name}` takes {arity} type argument{}, but {} {} written",
            if arity == 1 { "" } else { "s" },
            written.len(),
            if written.len() == 1 { "was" } else { "were" }
          ),
        )
        .with_secondary(declared, "declared here"),
      );
    }
    if arity == 0 {
      return Vec::new();
    }
    if let Some(hint) = self.literal_expectation(expected) {
      if let TypeKind::Named {
        def: other,
        ref args,
      } = *self.context.kind(hint)
      {
        if other == def && args.len() == arity {
          return args.clone();
        }
      }
    }
    (0..arity).map(|_| self.context.fresh_var()).collect()
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::errors::Diagnostics;
  use crate::{lexer, parser, resolve, Session};

  fn diagnose(source: &str) -> Diagnostics {
    let mut session = Session::new();
    let file = session.sources.add("test.pump", source.to_string());
    let text = source.to_string();

    let tokens = lexer::tokenize(file, &text, &mut session.diagnostics);
    let unit = parser::parse(
      file,
      vec!["test".to_string()],
      &tokens,
      &mut session.node_ids,
      &mut session.diagnostics,
    );

    let mut collected = Diagnostics::new();
    let resolution = resolve::resolve(
      vec![unit],
      std::path::Path::new("."),
      &mut session,
      &mut collected,
    )
    .expect("the entry unit always resolves");

    let mut from_checking = Diagnostics::new();
    let _ = check(resolution, &mut from_checking);

    let mut all = std::mem::take(&mut session.diagnostics);
    all.extend(collected);
    all.extend(from_checking);
    all
  }

  fn error_codes(source: &str) -> Vec<ErrorCode> {
    diagnose(source)
      .entries()
      .iter()
      .filter(|entry| entry.is_error())
      .map(|entry| entry.code)
      .collect()
  }

  fn in_main(body: &str) -> String {
    format!("fn main() {{\n{body}\n}}\n")
  }

  #[track_caller]
  fn assert_reports(source: &str, code: ErrorCode) {
    let found = error_codes(source);
    assert!(
      found.contains(&code),
      "expected {code:?}, found {found:?}\n--- source ---\n{source}"
    );
  }

  #[track_caller]
  fn assert_clean(source: &str) {
    let found = error_codes(source);
    assert!(
      found.is_empty(),
      "expected no errors, found {found:?}\n--- source ---\n{source}"
    );
  }

  #[test]
  fn print_accepts_anything_an_interpolation_accepts() {
    assert_clean(
      "fn main() {
  println(42)
  println(3.5)
  println(true)
  println('c')
  print(\"s\")
}
",
    );
  }

  #[test]
  fn the_operating_system_builtins_have_the_types_they_advertise() {
    assert_clean(
      "fn main() {
  let text: string? = read_file_text(\"a.txt\")
  let bytes: [int]? = read_file_bytes(\"a.bin\")
  let wrote: bool = write_file_text(\"a.txt\", \"hi\")
  let dumped: bool = write_file_bytes(\"a.bin\", [1, 2, 3])
  let arguments: [string] = os_args()
  let code: int? = os_run(\"linker\", [\"a.o\"])
  let why: string = os_error()
  println(len(arguments))
}
",
    );
  }

  #[test]
  fn reading_a_file_gives_an_optional_that_has_to_be_narrowed() {
    // Null la cach may cua vao tho bao that bai. Muon mot `string` thang
    // thi dung `io.read_text`, no `fail` ho.
    assert_reports(
      &in_main("let text: string = read_file_text(\"a.txt\")"),
      ErrorCode::TypeMismatch,
    );
  }

  #[test]
  fn writing_bytes_wants_an_array_of_int() {
    assert_reports(
      &in_main("write_file_bytes(\"a.bin\", \"not an array\")"),
      ErrorCode::TypeMismatch,
    );
    assert_reports(
      &in_main("write_file_text(42, \"text\")"),
      ErrorCode::TypeMismatch,
    );
  }

  #[test]
  fn the_argument_free_builtins_take_no_arguments() {
    assert_reports(
      &in_main("println(len(os_args(1)))"),
      ErrorCode::WrongArgumentCount,
    );
    assert_reports(
      &in_main("println(os_error(\"why\"))"),
      ErrorCode::WrongArgumentCount,
    );
    assert_reports(
      &in_main("println(read_file_text())"),
      ErrorCode::WrongArgumentCount,
    );
  }

  #[test]
  fn an_operating_system_builtin_may_be_shadowed_like_any_prelude_value() {
    // 2.5.1: may ten nay o prelude nen shadow duoc, khac han `int` hay
    // `Error`. Cai nay quan trong that: prelude vua phinh ra bay ten.
    assert_clean(
      "fn os_args(): int {
  return 7
}

fn main() {
  println(os_args())
}
",
    );
  }

  #[test]
  fn a_bounded_type_parameter_is_stringable() {
    assert_clean(
      "fn show<T: Stringable>(value: T) {
  println(\"{value}\")
}

fn main() {
  show(1)
}
",
    );
  }

  #[test]
  fn an_unbounded_type_parameter_is_not_stringable() {
    let found = error_codes(
      "fn show<T>(value: T) {
  println(\"{value}\")
}

fn main() {
  show(1)
}
",
    );
    assert!(
      found.contains(&ErrorCode::InvalidInterpolation),
      "{found:?}"
    );
  }

  #[test]
  fn a_map_setter_is_reachable_after_a_dot() {
    // `set` la tu khoa (2.3.2) nen method khong the ten la `set`, phai la
    // `insert`
    assert_clean(
      "fn main() {
  let m: [string: int] = {}
  m.insert(\"a\", 1)
  println(m.length)
}
",
    );
  }

  #[track_caller]
  fn assert_message_contains(source: &str, code: ErrorCode, fragment: &str) {
    let diagnostics = diagnose(source);
    let rendered: Vec<&str> = diagnostics
      .entries()
      .iter()
      .filter(|entry| entry.code == code)
      .map(|entry| entry.message.as_str())
      .collect();
    assert!(
      rendered.iter().any(|message| message.contains(fragment)),
      "no {code:?} message contained {fragment:?}; messages were {rendered:?}\
       \n--- source ---\n{source}"
    );
  }

  #[test]
  fn a_let_with_no_annotation_takes_the_initialiser_type() {
    assert_clean(&in_main(
      "let age = 18\nlet name = \"Minh\"\nprintln(name)\nprintln(age)",
    ));
  }

  #[test]
  fn a_const_binding_cannot_be_reassigned() {
    assert_reports(
      &in_main("const x = 10\nx = 20"),
      ErrorCode::CannotAssignToConst,
    );
  }

  #[test]
  fn a_let_binding_can_be_reassigned() {
    assert_clean(&in_main(
      "let count = 0\ncount = 10\ncount += 1\nprintln(count)",
    ));
  }

  #[test]
  fn a_for_binding_cannot_be_reassigned() {
    assert_reports(
      &in_main("for i in 0..10 {\ni = 3\n}"),
      ErrorCode::CannotAssignToLoopBinding,
    );
  }

  #[test]
  fn an_annotation_and_its_initialiser_must_agree() {
    assert_reports(&in_main("let age: int = \"x\""), ErrorCode::TypeMismatch);
  }

  #[test]
  fn int_and_float_never_mix_implicitly() {
    assert_reports(
      &in_main("let a: int = 1\nlet b: float = 2.0\nprintln(a + b)"),
      ErrorCode::NoImplicitConversion,
    );
  }

  #[test]
  fn an_explicit_conversion_bridges_the_two() {
    assert_clean(&in_main(
      "let a: int = 1\nlet b: float = 2.0\nprintln(float(a) + b)",
    ));
  }

  #[test]
  fn a_float_literal_never_adopts_an_integer_type() {
    assert_reports(&in_main("let a: int = 1.5"), ErrorCode::LiteralOutOfRange);
  }

  #[test]
  fn an_integer_literal_adopts_uint_from_its_context() {
    assert_clean(&in_main("let a: uint = 7\nprintln(a)"));
  }

  #[test]
  fn uint_cannot_be_negated() {
    assert_reports(
      &in_main("let a: uint = 7\nprintln(-a)"),
      ErrorCode::NegateUnsigned,
    );
  }

  #[test]
  fn char_has_no_arithmetic() {
    assert_reports(
      &in_main("let c = 'a'\nprintln(c + c)"),
      ErrorCode::CharArithmetic,
    );
  }

  #[test]
  fn bitwise_binds_two_integers_of_the_same_type() {
    assert_clean(&in_main("let a = 6\nlet b = 3\nprintln(a & b)"));
  }

  #[test]
  fn bitwise_rejects_a_float() {
    assert_reports(
      &in_main("let a = 1.5\nprintln(a & a)"),
      ErrorCode::BitwiseOnNonInteger,
    );
  }

  #[test]
  fn a_condition_must_be_bool() {
    assert_reports(
      &in_main("let n = 1\nif n {\nprintln(n)\n}"),
      ErrorCode::NoTruthiness,
    );
  }

  #[test]
  fn the_truthiness_error_suggests_a_comparison() {
    assert_message_contains(
      &in_main("let n = 1\nif n {\nprintln(n)\n}"),
      ErrorCode::NoTruthiness,
      "must be `bool`",
    );
  }

  #[test]
  fn null_is_assignable_only_to_an_optional() {
    assert_reports(&in_main("let u: int = null"), ErrorCode::TypeMismatch);
    assert_clean(&in_main("let u: int? = null\nprintln(u == null)"));
  }

  #[test]
  fn an_optional_field_needs_a_null_test_first() {
    let source = "\
struct User {
  name: string
}

fn main() {
  let u: User? = null
  println(u.name)
}
";
    assert_reports(source, ErrorCode::UnknownField);
  }

  #[test]
  fn a_null_test_narrows_the_then_branch() {
    let source = "\
struct User {
  name: string
}

fn main() {
  let u: User? = null
  if u != null {
    println(u.name)
  }
}
";
    assert_clean(source);
  }

  #[test]
  fn narrowing_chains_through_an_and() {
    let source = "\
struct User {
  name: string
  age: int
}

fn main() {
  let u: User? = null
  if u != null && u.age > 18 {
    println(u.name)
  }
}
";
    assert_clean(source);
  }

  #[test]
  fn an_early_return_narrows_the_rest_of_the_body() {
    let source = "\
struct User {
  name: string
}

fn main() {
  let u: User? = null
  if u == null {
    return
  }
  println(u.name)
}
";
    assert_clean(source);
  }

  #[test]
  fn narrowing_does_not_survive_an_assignment_in_the_region() {
    let source = "\
struct User {
  name: string
}

fn main() {
  let u: User? = null
  if u != null {
    u = null
    println(u.name)
  }
}
";
    assert_reports(source, ErrorCode::UnknownField);
  }

  #[test]
  fn a_bare_optional_does_not_narrow_on_a_field_path() {
    let source = "\
struct Node {
  next: Node?
  value: int
}

fn main() {
  let n = Node { next: null, value: 1 }
  if n.next != null {
    println(n.next.value)
  }
}
";
    assert_reports(source, ErrorCode::UnknownField);
  }

  #[test]
  fn postfix_question_needs_an_optional_return_type() {
    let source = "\
fn first(items: [int]): int {
  let head: int? = null
  return head?
}

fn main() {
  println(first([1]))
}
";
    assert_reports(source, ErrorCode::PropagateNullInNonOptional);
  }

  #[test]
  fn postfix_question_is_fine_in_an_optional_function() {
    let source = "\
fn first(items: [int]): int? {
  let head: int? = null
  return head?
}

fn main() {
  let value = first([1])
  println(value == null)
}
";
    assert_clean(source);
  }

  #[test]
  fn a_failable_call_must_be_consumed() {
    let source = "\
fn read(): string! {
  return \"data\"
}

fn main() {
  let text = read()
  println(text)
}
";
    assert_reports(source, ErrorCode::UnhandledError);
  }

  #[test]
  fn propagation_needs_a_failable_caller() {
    let source = "\
fn read(): string! {
  return \"data\"
}

fn main() {
  let text = read()!
  println(text)
}
";
    assert_reports(source, ErrorCode::PropagateErrorInNonFailable);
  }

  #[test]
  fn propagation_is_fine_inside_a_failable_function() {
    let source = "\
fn read(): string! {
  return \"data\"
}

fn load(): string! {
  let text = read()!
  return text
}

fn main() {
  let text = load() catch {
    return
  }
  println(text)
}
";
    assert_clean(source);
  }

  #[test]
  fn a_catch_block_must_diverge() {
    let source = "\
fn read(): string! {
  return \"data\"
}

fn main() {
  let text = read() catch {
    println(\"oops\")
  }
  println(text)
}
";
    assert_reports(source, ErrorCode::CatchBlockFallsThrough);
  }

  #[test]
  fn a_catch_value_supplies_a_fallback() {
    let source = "\
fn read(): string! {
  return \"data\"
}

fn main() {
  let text = read() catch \"\"
  println(text)
}
";
    assert_clean(source);
  }

  #[test]
  fn catch_on_something_that_cannot_fail_is_an_error() {
    assert_reports(
      &in_main("let n = 1 catch 2\nprintln(n)"),
      ErrorCode::CatchOnNonFailable,
    );
  }

  #[test]
  fn fail_needs_a_failable_function() {
    let source = "\
fn f(): string {
  fail \"nope\"
}

fn main() {
  println(f())
}
";
    assert_reports(source, ErrorCode::FailOutsideFailable);
  }

  #[test]
  fn a_caught_error_is_bound_with_a_message() {
    let source = "\
fn read(): string! {
  fail \"nope\"
}

fn main() {
  let text = read() catch e {
    println(e.message())
    return
  }
  println(text)
}
";
    assert_clean(source);
  }

  #[test]
  fn a_missing_argument_is_named() {
    let source = "\
fn connect(host: string, port: int) {
  println(host)
  println(port)
}

fn main() {
  connect(\"localhost\")
}
";
    assert_reports(source, ErrorCode::MissingArgument);
    assert_message_contains(source, ErrorCode::MissingArgument, "port");
  }

  #[test]
  fn a_default_fills_an_omitted_parameter() {
    let source = "\
fn connect(host: string, port: int = 80) {
  println(host)
  println(port)
}

fn main() {
  connect(\"localhost\")
}
";
    assert_clean(source);
  }

  #[test]
  fn named_arguments_bind_by_name() {
    let source = "\
fn connect(host: string, port: int = 80) {
  println(host)
  println(port)
}

fn main() {
  connect(host: \"localhost\", port: 8080)
}
";
    assert_clean(source);
  }

  #[test]
  fn naming_an_already_filled_parameter_is_an_error() {
    let source = "\
fn connect(host: string, port: int = 80) {
  println(host)
  println(port)
}

fn main() {
  connect(\"localhost\", host: \"other\")
}
";
    assert_reports(source, ErrorCode::ArgumentSuppliedTwice);
  }

  #[test]
  fn an_unknown_parameter_name_is_an_error() {
    let source = "\
fn connect(host: string) {
  println(host)
}

fn main() {
  connect(hostname: \"localhost\")
}
";
    assert_reports(source, ErrorCode::UnknownNamedArgument);
  }

  #[test]
  fn a_variadic_collects_the_trailing_arguments() {
    let source = "\
fn total(values: ...int): int {
  let sum = 0
  for value in values {
    sum += value
  }
  return sum
}

fn main() {
  println(total(1, 2, 3))
  println(total())
}
";
    assert_clean(source);
  }

  #[test]
  fn a_variadic_cannot_be_passed_by_name() {
    let source = "\
fn total(values: ...int): int {
  return 0
}

fn main() {
  println(total(values: 1))
}
";
    assert_reports(source, ErrorCode::VariadicPassedByName);
  }

  #[test]
  fn too_many_arguments_is_an_arity_error() {
    let source = "\
fn one(a: int) {
  println(a)
}

fn main() {
  one(1, 2)
}
";
    assert_reports(source, ErrorCode::WrongArgumentCount);
  }

  #[test]
  fn a_type_argument_is_inferred_from_the_arguments() {
    let source = "\
fn first<T>(items: [T]): T? {
  if items.length == 0 {
    return null
  }
  return items[0]
}

fn main() {
  let head = first([1, 2, 3])
  println(head == null)
}
";
    assert_clean(source);
  }

  #[test]
  fn a_turbofish_pins_the_instantiation() {
    let source = "\
fn identity<T>(value: T): T {
  return value
}

fn main() {
  println(identity::<int>(1))
}
";
    assert_clean(source);
  }

  #[test]
  fn a_turbofish_of_the_wrong_arity_is_an_error() {
    let source = "\
fn identity<T>(value: T): T {
  return value
}

fn main() {
  println(identity::<int, string>(1))
}
";
    assert_reports(source, ErrorCode::WrongTypeArgumentCount);
  }

  #[test]
  fn a_generic_struct_infers_from_its_fields() {
    let source = "\
struct Box<T> {
  value: T
}

fn main() {
  let b = Box { value: 1 }
  println(b.value)
}
";
    assert_clean(source);
  }

  #[test]
  fn an_unbounded_type_parameter_has_no_methods() {
    let source = "\
fn describe<T>(value: T): string {
  return value.to_string()
}

fn main() {
  println(describe(1))
}
";
    assert_reports(source, ErrorCode::MethodOnUnboundedGeneric);
  }

  #[test]
  fn structural_conformance_satisfies_an_interface() {
    let source = "\
interface Printable {
  fn describe(): string
}

struct User {
  name: string

  fn describe(): string {
    return name
  }
}

implements User: Printable

fn show(item: Printable) {
  println(item.describe())
}

fn main() {
  show(User { name: \"Minh\" })
}
";
    assert_clean(source);
  }

  #[test]
  fn a_missing_method_fails_the_assertion() {
    let source = "\
interface Printable {
  fn describe(): string
}

struct User {
  name: string
}

implements User: Printable

fn main() {
  println(User { name: \"Minh\" }.name)
}
";
    assert_reports(source, ErrorCode::InterfaceNotSatisfied);
  }

  #[test]
  fn a_mismatched_signature_fails_the_assertion() {
    let source = "\
interface Printable {
  fn describe(): string
}

struct User {
  name: string

  fn describe(): int {
    return 1
  }
}

implements User: Printable

fn main() {
  println(User { name: \"Minh\" }.describe())
}
";
    assert_reports(source, ErrorCode::InterfaceNotSatisfied);
  }

  #[test]
  fn a_field_resolves_with_no_prefix_inside_a_method() {
    let source = "\
struct User {
  name: string

  fn greet() {
    println(\"Hello \" + name)
  }
}

fn main() {
  User { name: \"Minh\" }.greet()
}
";
    assert_clean(source);
  }

  #[test]
  fn a_parameter_shadows_a_field_and_this_reaches_it() {
    let source = "\
struct User {
  name: string

  fn rename(name: string) {
    this.name = name
  }
}

fn main() {
  let u = User { name: \"Minh\" }
  u.rename(\"Linh\")
  println(u.name)
}
";
    assert_clean(source);
  }

  #[test]
  fn a_struct_literal_must_give_every_field() {
    let source = "\
struct User {
  name: string
  age: int
}

fn main() {
  let u = User { name: \"Minh\" }
  println(u.name)
}
";
    assert_reports(source, ErrorCode::MissingStructField);
    assert_message_contains(source, ErrorCode::MissingStructField, "age");
  }

  #[test]
  fn an_unknown_field_in_a_literal_is_named() {
    let source = "\
struct User {
  name: string
}

fn main() {
  let u = User { name: \"Minh\", age: 18 }
  println(u.name)
}
";
    assert_reports(source, ErrorCode::UnknownStructField);
  }

  #[test]
  fn a_field_of_a_const_binding_stays_assignable() {
    let source = "\
struct User {
  name: string
  age: int
}

fn main() {
  const u = User { name: \"Minh\", age: 18 }
  u.age = 19
  println(u.age)
}
";
    assert_clean(source);
  }

  #[test]
  fn an_empty_collection_literal_needs_an_annotation() {
    assert_reports(&in_main("let items = []"), ErrorCode::CannotInferType);
  }

  #[test]
  fn an_annotated_empty_collection_is_fine() {
    assert_clean(&in_main(
      "let items: [int] = []\nlet users: [string: int] = {}\nprintln(items.length + users.length)",
    ));
  }

  #[test]
  fn float_may_not_be_a_map_key() {
    assert_reports(
      &in_main("let m: [float: int] = {}\nprintln(m.length)"),
      ErrorCode::FloatNotHashable,
    );
  }

  #[test]
  fn a_string_is_not_indexable() {
    assert_reports(
      &in_main("let s = \"hi\"\nprintln(s[0])"),
      ErrorCode::StringNotIndexable,
    );
  }

  #[test]
  fn a_map_index_yields_the_value_type() {
    assert_clean(&in_main(
      "let m: [string: int] = {}\nlet n: int = m[\"a\"]\nprintln(n)",
    ));
  }

  #[test]
  fn a_tuple_element_is_reached_by_position() {
    assert_clean(&in_main("let p: (int, int) = (10, 20)\nprintln(p.0 + p.1)"));
  }

  #[test]
  fn a_tuple_index_past_the_end_is_an_error() {
    assert_reports(
      &in_main("let p: (int, int) = (10, 20)\nprintln(p.2)"),
      ErrorCode::TupleIndexOutOfRange,
    );
  }

  #[test]
  fn a_match_over_an_enum_must_be_exhaustive() {
    let source = "\
enum Color {
  Red
  Green
  Blue
}

fn main() {
  let c = Color.Red
  match c {
    Color.Red => println(\"red\")
    Color.Green => println(\"green\")
  }
}
";
    assert_reports(source, ErrorCode::NonExhaustiveMatch);
    assert_message_contains(source, ErrorCode::NonExhaustiveMatch, "does not cover");
  }

  #[test]
  fn the_missing_variant_is_named() {
    let source = "\
enum Color {
  Red
  Green
  Blue
}

fn main() {
  let c = Color.Red
  match c {
    Color.Red => println(\"red\")
    Color.Green => println(\"green\")
  }
}
";
    let diagnostics = diagnose(source);
    let notes: Vec<String> = diagnostics
      .entries()
      .iter()
      .filter(|entry| entry.code == ErrorCode::NonExhaustiveMatch)
      .flat_map(|entry| entry.notes.clone())
      .collect();
    assert!(
      notes.iter().any(|note| note.contains("Color.Blue")),
      "expected the missing variant to be named, notes were {notes:?}"
    );
  }

  #[test]
  fn covering_every_variant_is_exhaustive() {
    let source = "\
enum Color {
  Red
  Green
  Blue
}

fn main() {
  let c = Color.Red
  match c {
    Color.Red => println(\"red\")
    Color.Green => println(\"green\")
    Color.Blue => println(\"blue\")
  }
}
";
    assert_clean(source);
  }

  #[test]
  fn an_integer_match_needs_a_wildcard() {
    assert_reports(
      &in_main("let n = 1\nmatch n {\n0 => println(\"zero\")\n1 => println(\"one\")\n}"),
      ErrorCode::NonExhaustiveMatch,
    );
  }

  #[test]
  fn a_wildcard_makes_any_match_exhaustive() {
    assert_clean(&in_main(
      "let n = 1\nmatch n {\n0 => println(\"zero\")\n_ => println(\"other\")\n}",
    ));
  }

  #[test]
  fn an_arm_covered_by_an_earlier_one_is_unreachable() {
    assert_reports(
      &in_main("let n = 1\nmatch n {\n_ => println(\"any\")\n0 => println(\"zero\")\n}"),
      ErrorCode::UnreachableMatchArm,
    );
  }

  #[test]
  fn a_bool_match_is_exhaustive_from_true_and_false() {
    assert_clean(&in_main(
      "let b = true\nmatch b {\ntrue => println(\"yes\")\nfalse => println(\"no\")\n}",
    ));
  }

  #[test]
  fn an_optional_match_needs_null_and_a_value_pattern() {
    assert_reports(
      &in_main("let n: int? = null\nmatch n {\nnull => println(\"none\")\n}"),
      ErrorCode::NonExhaustiveMatch,
    );
    assert_clean(&in_main(
      "let n: int? = null\nmatch n {\nnull => println(\"none\")\nv => println(v == null)\n}",
    ));
  }

  #[test]
  fn a_binding_takes_the_optional_type_but_a_literal_looks_through_it() {
    assert_reports(
      &in_main(
        "let n: int? = null\nmatch n {\nnull => println(\"none\")\nv => println(v)\n}",
      ),
      ErrorCode::InvalidInterpolation,
    );
    assert_clean(&in_main(
      "let n: int? = null\nmatch n {\nnull => println(\"none\")\n0 => println(\"zero\")\n_ => println(\"other\")\n}",
    ));
  }

  #[test]
  fn a_payload_pattern_binds_at_the_payload_type() {
    let source = "\
enum Shape {
  Circle(int)
  Rect(int, int)
}

fn main() {
  let s = Shape.Circle(3)
  match s {
    Shape.Circle(r) => println(r)
    Shape.Rect(w, h) => println(w * h)
  }
}
";
    assert_clean(source);
  }

  #[test]
  fn a_payload_arity_mismatch_is_reported() {
    let source = "\
enum Shape {
  Rect(int, int)
}

fn main() {
  let s = Shape.Rect(1, 2)
  match s {
    Shape.Rect(w) => println(w)
  }
}
";
    assert_reports(source, ErrorCode::WrongArgumentCount);
  }

  #[test]
  fn a_guarded_arm_does_not_make_a_match_exhaustive() {
    let source = "\
enum Color {
  Red
  Green
}

fn main() {
  let c = Color.Red
  match c {
    Color.Red => println(\"red\")
    Color.Green if 1 > 0 => println(\"green\")
  }
}
";
    assert_reports(source, ErrorCode::NonExhaustiveMatch);
  }

  #[test]
  fn nested_payload_patterns_are_checked_for_exhaustiveness() {
    let source = "\
enum Inner {
  A
  B
}

enum Outer {
  Wrap(Inner)
}

fn main() {
  let o = Outer.Wrap(Inner.A)
  match o {
    Outer.Wrap(Inner.A) => println(\"a\")
  }
}
";
    assert_reports(source, ErrorCode::NonExhaustiveMatch);
  }

  #[test]
  fn an_or_pattern_must_bind_the_same_names() {
    let source = "\
enum Pair {
  Left(int)
  Right(int)
}

fn main() {
  let p = Pair.Left(1)
  match p {
    Pair.Left(x) | Pair.Right(y) => println(x)
  }
}
";
    assert_reports(source, ErrorCode::OrPatternBindingMismatch);
  }

  #[test]
  fn an_or_pattern_binding_the_same_name_is_fine() {
    let source = "\
enum Pair {
  Left(int)
  Right(int)
}

fn main() {
  let p = Pair.Left(1)
  match p {
    Pair.Left(x) | Pair.Right(x) => println(x)
  }
}
";
    assert_clean(source);
  }

  #[test]
  fn a_range_pattern_never_makes_an_integer_match_exhaustive() {
    assert_reports(
      &in_main("let n = 1\nmatch n {\n0..=9 => println(\"digit\")\n}"),
      ErrorCode::NonExhaustiveMatch,
    );
  }

  #[test]
  fn a_reversed_range_pattern_is_rejected() {
    assert_reports(
      &in_main(
        "let n = 1\nmatch n {\n9..=0 => println(\"never\")\n_ => println(\"other\")\n}",
      ),
      ErrorCode::InvalidRangePattern,
    );
  }

  #[test]
  fn a_struct_pattern_without_dots_must_list_every_field() {
    let source = "\
struct Point {
  x: int
  y: int
}

fn main() {
  let p = Point { x: 1, y: 2 }
  match p {
    Point { x: a } => println(a)
  }
}
";
    assert_reports(source, ErrorCode::MissingStructField);
  }

  #[test]
  fn a_struct_pattern_with_dots_and_shorthand_binds() {
    let source = "\
struct Point {
  x: int
  y: int
}

fn main() {
  let p = Point { x: 1, y: 2 }
  match p {
    Point { x, .. } => println(x)
  }
}
";
    assert_clean(source);
  }

  #[test]
  fn every_path_must_return_a_value() {
    let source = "\
fn max(a: int, b: int): int {
  if a > b {
    return a
  }
}

fn main() {
  println(max(1, 2))
}
";
    assert_reports(source, ErrorCode::MissingReturn);
  }

  #[test]
  fn both_branches_returning_satisfies_the_check() {
    let source = "\
fn max(a: int, b: int): int {
  if a > b {
    return a
  }
  return b
}

fn main() {
  println(max(1, 2))
}
";
    assert_clean(source);
  }

  #[test]
  fn an_exhaustive_match_can_be_the_only_return() {
    let source = "\
enum Color {
  Red
  Green
}

fn name(c: Color): string {
  match c {
    Color.Red => {
      return \"red\"
    }
    Color.Green => {
      return \"green\"
    }
  }
}

fn main() {
  println(name(Color.Red))
}
";
    assert_clean(source);
  }

  #[test]
  fn a_void_function_may_not_return_a_value() {
    let source = "\
fn f() {
  return 1
}

fn main() {
  f()
}
";
    assert_reports(source, ErrorCode::ReturnValueInVoidFunction);
  }

  #[test]
  fn a_closure_takes_its_declared_types() {
    assert_clean(&in_main(
      "let add = fn(a: int, b: int): int {\nreturn a + b\n}\nprintln(add(1, 2))",
    ));
  }

  #[test]
  fn a_closure_call_checks_its_arguments() {
    assert_reports(
      &in_main(
        "let add = fn(a: int, b: int): int {\nreturn a + b\n}\nprintln(add(1, \"x\"))",
      ),
      ErrorCode::TypeMismatch,
    );
  }

  #[test]
  fn a_closure_call_takes_no_named_arguments() {
    assert_reports(
      &in_main("let add = fn(a: int): int {\nreturn a\n}\nprintln(add(a: 1))"),
      ErrorCode::NamedArgumentThroughValue,
    );
  }

  #[test]
  fn a_closure_captures_an_enclosing_binding() {
    assert_clean(&in_main(
      "let base = 10\nlet bump = fn(n: int): int {\nreturn n + base\n}\nprintln(bump(1))",
    ));
  }

  #[test]
  fn interpolation_accepts_a_primitive() {
    assert_clean(&in_main(
      "let name = \"Minh\"\nlet age = 18\nprintln(\"{name} is {age}\")",
    ));
  }

  #[test]
  fn interpolation_rejects_a_type_without_to_string() {
    let source = "\
struct User {
  name: string
}

fn main() {
  let u = User { name: \"Minh\" }
  println(\"{u}\")
}
";
    assert_reports(source, ErrorCode::InvalidInterpolation);
  }

  #[test]
  fn interpolation_accepts_a_stringable_type() {
    let source = "\
struct User {
  name: string

  fn to_string(): string {
    return name
  }
}

fn main() {
  let u = User { name: \"Minh\" }
  println(\"{u}\")
}
";
    assert_clean(source);
  }

  #[test]
  fn a_range_iterates_as_int() {
    assert_clean(&in_main("for i in 0..10 {\nprintln(i + 1)\n}"));
  }

  #[test]
  fn a_map_iterates_as_a_key_value_tuple() {
    assert_clean(&in_main(
      "let m: [string: int] = {}\nfor entry in m {\nprintln(entry.0)\nprintln(entry.1)\n}",
    ));
  }

  #[test]
  fn an_int_cannot_be_iterated() {
    assert_reports(
      &in_main("for i in 3 {\nprintln(i)\n}"),
      ErrorCode::NotIterable,
    );
  }

  #[test]
  fn length_is_a_field_and_takes_no_parentheses() {
    assert_clean(&in_main("let items: [int] = []\nprintln(items.length)"));
    assert_reports(
      &in_main("let items: [int] = []\nprintln(items.length())"),
      ErrorCode::NotCallable,
    );
  }

  #[test]
  fn array_methods_have_the_element_type() {
    assert_clean(&in_main(
      "let items: [int] = []\nitems.push(1)\nprintln(items.pop() + 1)",
    ));
    assert_reports(
      &in_main("let items: [int] = []\nitems.push(\"x\")"),
      ErrorCode::TypeMismatch,
    );
  }

  #[test]
  fn an_unknown_method_lists_what_is_available() {
    let source = "\
struct User {
  name: string

  fn greet() {
    println(name)
  }
}

fn main() {
  User { name: \"Minh\" }.shout()
}
";
    assert_reports(source, ErrorCode::UnknownField);
  }

  #[test]
  fn optional_expect_unwraps_without_narrowing() {
    assert_clean(&in_main(
      "let n: int? = null\nprintln(n.expect(\"needed\") + 1)\nprintln(n.or(0))",
    ));
  }

  #[test]
  fn the_specification_program_checks() {
    let source = "\
const PORT: int = 8080

struct User {
  name: string
  age: int

  fn greet() {
    println(\"Hello \" + name)
  }
}

fn create_user(name: string, age: int): User {
  let user = User {
    name: name
    age: age
  }

  return user
}

fn main() {
  println(PORT)

  let user = create_user(\"Minh\", 18)

  if user.age >= 18 {
    user.greet()
  } else {
    println(\"Minor\")
  }

  for i in 0..10 {
    println(i)
  }

  return
}
";
    assert_clean(source);
  }

  #[test]
  fn no_inference_variable_survives_into_the_handoff() {
    let source = "\
fn first<T>(items: [T]): T? {
  if items.length == 0 {
    return null
  }
  return items[0]
}

fn main() {
  let head = first([1, 2, 3])
  let word = first([\"a\"])
  println(head == null)
  println(word == null)
}
";
    let mut session = Session::new();
    let file = session.sources.add("test.pump", source.to_string());
    let text = source.to_string();
    let tokens = lexer::tokenize(file, &text, &mut session.diagnostics);
    let unit = parser::parse(
      file,
      vec!["test".to_string()],
      &tokens,
      &mut session.node_ids,
      &mut session.diagnostics,
    );
    let mut collected = Diagnostics::new();
    let resolution = resolve::resolve(
      vec![unit],
      std::path::Path::new("."),
      &mut session,
      &mut collected,
    )
    .expect("the entry unit always resolves");
    let checked = check(resolution, &mut collected).expect("the program checks");

    assert!(!collected.has_errors(), "{:?}", error_codes(source));
    for (&node, &ty) in &checked.expression_types {
      assert!(
        !matches!(checked.context().kind(ty), TypeKind::Var(_)),
        "node {node:?} kept an inference variable"
      );
    }
    // moi bo doi so kieu khac nhau la mot ban the hoa (D-21)
    let generic: Vec<&Instantiation> = checked
      .instantiations
      .iter()
      .filter(|entry| checked.context().func(entry.func).name == "first")
      .collect();
    assert_eq!(generic.len(), 2, "found {generic:?}");
  }
}
