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
        // gia tri interface ...
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
        None => out.push(format!( "missing method `{}`", self.describe_signature(slot) )),
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
      let Some(decl) = do_at(&units, location) else {
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

fn do_at(units: &[SourceUnit], location: FuncLocation) -> Option<&FunctionDecl> {
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
        // cai nay `while true` khong
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

// Tra ve cho GAN dau tien vao
// cai nay truoc may ham
// bao loi phai ...
// cai nay biet cai gi
