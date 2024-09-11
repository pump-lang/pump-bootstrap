// giai ten. Nap import, dien may
// dung cai ma no goi ten.
//
// duong dan import la mot file nam duoi goc project, ma goc project la thu
// muc chua file entry. Mot file la mot module, khong hon.
//
// khai bao o muc cao nhat khong quan
// cai nay hai luot: gom
//
// ten kieu co ...
//  mot kieu ...
//
// cai nay module so 0
// ngon ngu can: interface Error, interface Stringable, va struct Range. No
// deo theo mot SourceUnit rong chi de `units` con tra duoc theo ModuleId.
// cai nay file entry la
//
// Ban dau t viet ca file nay trong mot
// borrow checker keu ...

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
