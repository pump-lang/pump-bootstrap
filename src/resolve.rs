// giai ten. Nap import, dien may bang khai bao, buoc tung identifier vao
// dung cai ma no goi ten.
//
// duong dan import la mot file nam duoi goc project, ma goc project la thu
// muc chua file entry. Mot file la mot module, khong hon.
//
// khai bao o muc cao nhat khong quan tam thu tu (10.1.2) nen cho nay chay
// hai luot: gom het ten truoc, sau do moi giai kieu trong tung chu ky.
//
// ten kieu co khong gian ten rieng, nen mot bien `x` khong bao gio che mat
// mot kieu ten `x`.
//
// module so 0 la module t bia ra, ten `<builtin>`, giu ba thu ma ban than
// ngon ngu can: interface Error, interface Stringable, va struct Range. No
// deo theo mot SourceUnit rong chi de `units` con tra duoc theo ModuleId.
// File entry la module 1.
//
// Ban dau t viet ca file nay trong mot ham. Sau moi tach ra, va cho nao ma
// borrow checker keu thi t clone(). Biet la phi nhung chay duoc.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::ast::{
    Argument, Block, CatchHandler, ClosureExpr, ConstDecl, Declaration, EnumDecl, EnumMember, Expr,
    ExprKind, FunctionDecl, GenericParam, Ident, InterfaceDecl, IrrefutablePattern,
    IrrefutablePatternKind, MatchArmBody, NodeId, Param, ParamKind, Pattern, PatternKind,
    SourceUnit, Stmt, StmtKind, StringPart, StructDecl, StructMember, TypeExpr, TypeExprKind,
    TypePath, VisibilityKind,
};
use crate::errors::{CompileError, Diagnostics, ErrorCode};
use crate::token::Span;
use crate::types::{
    ConstValue, DefId, EnumDef, FieldDef, FnType, FuncDef, FuncId, GenericId, GenericOwner,
    GenericParamDef, InterfaceDef, ModuleId, ParamDef, StructDef, TypeContext, TypeDef,
    TypeDefKind, TypeId, TypeKind, VariantDef,
};

// ban giao sang cho checker

/// What the resolver gives to the checker.
#[derive(Debug)]
pub struct Resolution {
    pub context: TypeContext,
    pub units: Vec<SourceUnit>,
    pub root_module: ModuleId,
    pub entry: Option<FuncId>,
    pub values: HashMap<NodeId, ValueBinding>,
    pub types: HashMap<NodeId, TypeId>,
    pub locals: Vec<LocalBinding>,
    pub const_init_order: Vec<GlobalConstId>,

    pub globals: Vec<GlobalConst>,
    pub declared_locals: HashMap<Span, LocalId>,
    pub closures: HashMap<NodeId, ClosureInfo>,
    pub functions: HashMap<FuncId, FuncLocation>,
    pub implements: Vec<ImplementsAssertion>,
    pub pattern_defs: HashMap<NodeId, DefId>,
    pub prelude: Prelude,
}

impl Resolution {
    /// Khai bao cua ham hoac method. None cho chu ky method cua interface,
    /// vi cai do khong co than.
    pub fn function_decl(&self, func: FuncId) -> Option<&FunctionDecl> {
        let location = self.functions.get(&func)?;
        let unit = self.units.get(location.module.index())?;
        let declaration = unit.declarations.get(location.declaration)?;
        match (declaration, location.member) {
            (Declaration::Function(decl), None) => Some(decl),
            (Declaration::Struct(decl), Some(index)) => match decl.members.get(index)? {
                StructMember::Method(method) => Some(method),
                StructMember::Field(_) => None,
            },
            (Declaration::Enum(decl), Some(index)) => match decl.members.get(index)? {
                EnumMember::Method(method) => Some(method),
                EnumMember::Variant(_) => None,
            },
            _ => None,
        }
    }

    /// Khai bao cua mot hang o muc module.
    pub fn global_decl(&self, global: GlobalConstId) -> Option<&ConstDecl> {
        let entry = self.globals.get(global.index())?;
        let unit = self.units.get(entry.module.index())?;
        match unit.declarations.get(entry.declaration)? {
            Declaration::Const(decl) => Some(decl),
            _ => None,
        }
    }

    pub fn local(&self, id: LocalId) -> &LocalBinding {
        &self.locals[id.index()]
    }
}

/// Id of one local binding: `let`, `const` trong block, tham so, hoac mot
/// ten do pattern buoc ra.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct LocalId(pub u32);

impl LocalId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Id of one constant at module level.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct GlobalConstId(pub u32);

impl GlobalConstId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// One local binding.
#[derive(Clone, Debug)]
pub struct LocalBinding {
    pub id: LocalId,
    pub name: String,
    pub reassignable: bool,
    pub captured: bool,
    pub span: Span,
    pub origin: LocalOrigin,
}

/// Which form of binding made this local.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LocalOrigin {
    Let,
    Const,
    Parameter,
    LoopBinding,
    PatternBinding,
    CatchBinding,
}

/// One constant at module level.
#[derive(Clone, Debug)]
pub struct GlobalConst {
    pub id: GlobalConstId,
    pub name: String,
    pub module: ModuleId,
    pub visibility: VisibilityKind,
    pub declaration: usize,
    pub path: Vec<u32>,
    pub span: Span,
}

/// Cho ma khai bao cua mot ham nam.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FuncLocation {
    pub module: ModuleId,
    pub declaration: usize,
    pub member: Option<usize>,
}

/// What a closure grabs from the scopes around it.
#[derive(Clone, Debug, Default)]
pub struct ClosureInfo {
    pub captures: Vec<LocalId>,
    pub captures_this: bool,
}

/// One `implements Subject: A, B`. 10.4.
#[derive(Clone, Debug)]
pub struct ImplementsAssertion {
    pub subject: DefId,
    pub subject_span: Span,
    pub interfaces: Vec<(DefId, Span)>,
    pub span: Span,
}

/// The definitions the language itself leans on. Khai bao san.
#[derive(Clone, Copy, Debug)]
pub struct Prelude {
    pub module: ModuleId,
    pub error: DefId,
    pub stringable: DefId,
    pub range: DefId,
    pub error_type: TypeId,
    pub range_type: TypeId,
}

/// What an identifier in expression position names, sau khi di het scope
/// theo 16.1.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ValueBinding {
    Local(LocalId),
    Captured(LocalId),
    Field { owner: DefId, index: u32 },
    Method(FuncId),
    Function(FuncId),
    GlobalConst(GlobalConstId),
    Module(ModuleId),
    Type(DefId),
    Predeclared(Predeclared),
    Conversion(TypeId),
}

/// Prelude values that may be shadowed. 2.5.1.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Predeclared {
    Print,
    Println,
    Panic,
    Assert,
    Len,
    // May cai duoi day la cua vao he dieu hanh, tho, chua boc gi ca. `std/io`
    // voi `std/os` moi la be mat de dung: chung doc cai o loi kieu errno cua
    // cai nay runtime roi doi
    // cai nay de khong ai
    ReadFileText,
    ReadFileBytes,
    WriteFileText,
    WriteFileBytes,
    OsArgs,
    OsRun,
    OsError,
}

impl Predeclared {
    pub fn spelling(self) -> &'static str {
        match self {
            Predeclared::Print => "print",
            Predeclared::Println => "println",
            Predeclared::Panic => "panic",
            Predeclared::Assert => "assert",
            Predeclared::Len => "len",
            Predeclared::ReadFileText => "read_file_text",
            Predeclared::ReadFileBytes => "read_file_bytes",
            Predeclared::WriteFileText => "write_file_text",
            Predeclared::WriteFileBytes => "write_file_bytes",
            Predeclared::OsArgs => "os_args",
            Predeclared::OsRun => "os_run",
            Predeclared::OsError => "os_error",
        }
    }
}

fn bundled2(segments: &[String]) -> Option<&'static str> {
    let path: Vec<&str> = segments.iter().map(String::as_str).collect();
    match path.as_slice() {
        ["io"] => Some(include_str!("../std/io.pump")),
        ["io", "raw"] => Some(include_str!("../std/io/raw.pump")),
        ["os"] => Some(include_str!("../std/os.pump")),
        ["strings"] => Some(include_str!("../std/strings.pump")),
        _ => None,
    }
}

/// Ten khai bao san ma khong duoc che. 2.5.1.
pub const NON_SHADOWABLE: [&str; 8] = [
    "bool", "int", "uint", "float", "char", "string", "void", "Error",
];

fn do_value(name: &str) -> Option<Predeclared> {
    match name {
        "print" => Some(Predeclared::Print),
        "println" => Some(Predeclared::Println),
        "panic" => Some(Predeclared::Panic),
        "assert" => Some(Predeclared::Assert),
        "len" => Some(Predeclared::Len),
        "read_file_text" => Some(Predeclared::ReadFileText),
        "read_file_bytes" => Some(Predeclared::ReadFileBytes),
        "write_file_text" => Some(Predeclared::WriteFileText),
        "write_file_bytes" => Some(Predeclared::WriteFileBytes),
        "os_args" => Some(Predeclared::OsArgs),
        "os_run" => Some(Predeclared::OsRun),
        "os_error" => Some(Predeclared::OsError),
        _ => None,
    }
}

fn primitive_type(name: &str) -> Option<TypeId> {
    match name {
        "bool" => Some(TypeId::BOOL),
        "int" => Some(TypeId::INT),
        "uint" => Some(TypeId::UINT),
        "float" => Some(TypeId::FLOAT),
        "char" => Some(TypeId::CHAR),
        "string" => Some(TypeId::STRING),
        "void" => Some(TypeId::VOID),
        _ => None,
    }
}

fn conversion2(name: &str) -> Option<TypeId> {
    match name {
        "int" => Some(TypeId::INT),
        "uint" => Some(TypeId::UINT),
        "float" => Some(TypeId::FLOAT),
        "char" => Some(TypeId::CHAR),
        "string" => Some(TypeId::STRING),
        _ => None,
    }
}

// cua vao

/// Resolve the entry file and everything it pulls in.
pub fn resolve2(
    units: Vec<SourceUnit>,
    project_root: &Path,
    session: &mut crate::Session,
    diagnostics: &mut Diagnostics,
) -> Result<Resolution, CompileError> {
    let Some(entry_unit) = units.into_iter().next() else {
        return Err(CompileError::at(
            ErrorCode::EntryFileNotFound,
            Span::synthetic(),
            "the compilation has no entry unit",
        ));
    };

    let mut resolver = Resolver::new(diagnostics);
    resolver.build_prelude();
    resolver.load_graph(entry_unit, project_root, session);
    resolver.declare_items();
    resolver.resolve_generic_parameters();
    resolver.resolve_type_bodies();
    resolver.resolve_signatures();
    resolver.resolve_implements();
    resolver.fold_parameter_defaults();
    resolver.resolve_bodies();
    resolver.order_module_constants();
    resolver.check_entry_point();
    resolver.report_unused_imports();
    Ok(resolver.finish())
}

// trang thai
