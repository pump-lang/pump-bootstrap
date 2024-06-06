// cay cu phap.
//
// ba luat t tu dat cho ca file nay:
//
//  1. node nao cung phai co Span, khong tru cai nao. Node do compiler tu che
//     ra thi dung Span::synthetic().
//  2. member giu nguyen thu tu trong source. Field voi method cua struct nam
//     chung mot Vec chu khong tach hai list.
//  3. cay ...
// cai nay  khong co
//     NodeId.

#![allow(dead_code)]

use crate::token::Span;

/// Id of a node, cho may pha sau con ghi chu vao duoc.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct NodeId(pub u32);

impl NodeId {
    /// Cho node parser tu che ra ma khong xin id.
    pub const NONE: NodeId = NodeId(u32::MAX);

    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Gives out NodeId. Mot cai cho ca lan bien dich.
#[derive(Clone, Debug, Default)]
pub struct NodeIdAllocator {
    next: u32,
}

impl NodeIdAllocator {
    pub fn new() -> NodeIdAllocator {
        NodeIdAllocator::default()
    }

    pub fn allocate(&mut self) -> NodeId {
        let id = NodeId(self.next);
        self.next += 1;
        id
    }

    /// How many ids given out so far. Bang side table dai dung bang nay.
    pub fn count(&self) -> usize {
        self.next as usize
    }
}

/// One identifier the way it was written, with its span.
#[derive(Clone, Debug, PartialEq)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

impl Ident {
    pub fn new(name: impl Into<String>, span: Span) -> Ident {
        Ident {
            name: name.into(),
            span,
        }
    }
}

/// Visibility as declared, grammar 10.3.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VisibilityKind {
    Private,
    Public,
}

/// Visibility plus the span of the word that set it.
#[derive(Clone, Debug, PartialEq)]
pub struct Visibility {
    pub kind: VisibilityKind,
    pub span: Option<Span>,
}

impl Visibility {
    pub fn implicit_private() -> Visibility {
        Visibility {
            kind: VisibilityKind::Private,
            span: None,
        }
    }

    pub fn is_public(&self) -> bool {
        self.kind == VisibilityKind::Public
    }
}

impl Default for Visibility {
    fn default() -> Visibility {
        Visibility::implicit_private()
    }
}

// ===== cau truc file (10) =====

/// One .pump file.
#[derive(Clone, Debug)]
pub struct SourceUnit {
    pub id: NodeId,
    pub module_path: Vec<String>,
    pub imports: Vec<Import>,
    pub declarations: Vec<Declaration>,
    pub span: Span,
}

/// `import net\http as http` (grammar 10.2).
#[derive(Clone, Debug)]
pub struct Import {
    pub id: NodeId,
    pub path: Vec<Ident>,
    pub alias: Option<Ident>,
    pub span: Span,
}

impl Import {
    /// Ten ma import nay tao ra ben file goi, 10.2.5.
    pub fn bound_name(&self) -> &Ident {
        self.alias
            .as_ref()
            .unwrap_or_else(|| self.path.last().unwrap())
    }
}

/// A top-level declaration (grammar 10.3).
#[derive(Clone, Debug)]
pub enum Declaration {
    Function(FunctionDecl),
    Struct(StructDecl),
    Enum(EnumDecl),
    Interface(InterfaceDecl),
    Const(ConstDecl),
    Implements(ImplementsDecl),
}

impl Declaration {
    pub fn span(&self) -> Span {
        match self {
            Declaration::Function(decl) => decl.span,
            Declaration::Struct(decl) => decl.span,
            Declaration::Enum(decl) => decl.span,
            Declaration::Interface(decl) => decl.span,
            Declaration::Const(decl) => decl.span,
            Declaration::Implements(decl) => decl.span,
        }
    }

    /// Ten khai bao. None cho `implements` vi no khong dat ten gi moi.
    pub fn name(&self) -> Option<&Ident> {
        match self {
            Declaration::Function(decl) => Some(&decl.name),
            Declaration::Struct(decl) => Some(&decl.name),
            Declaration::Enum(decl) => Some(&decl.name),
            Declaration::Interface(decl) => Some(&decl.name),
            Declaration::Const(_) => None,
            Declaration::Implements(_) => None,
        }
    }
}

/// `implements User: Printable, Comparable` (grammar 10.4).
#[derive(Clone, Debug)]
pub struct ImplementsDecl {
    pub id: NodeId,
    pub subject: Ident,
    pub interfaces: Vec<TypePath>,
    pub span: Span,
}

// ===== khai bao (12) =====

/// A function with a name, a method, or the signature half of a closure.
#[derive(Clone, Debug)]
pub struct FunctionDecl {
    pub id: NodeId,
    pub visibility: Visibility,
    pub name: Ident,
    pub generics: Vec<GenericParam>,
    pub params: Vec<Param>,
    pub return_type: Option<TypeExpr>,
    pub body: Block,
    pub span: Span,
}

/// An interface method signature (grammar 12.4).
#[derive(Clone, Debug)]
pub struct InterfaceMethod {
    pub id: NodeId,
    pub name: Ident,
    pub generics: Vec<GenericParam>,
    pub params: Vec<Param>,
    pub return_type: Option<TypeExpr>,
    pub span: Span,
}

/// `T` or `T: Printable + Comparable` (grammar 12.1).
#[derive(Clone, Debug)]
pub struct GenericParam {
    pub id: NodeId,
    pub name: Ident,
    pub bounds: Vec<TypePath>,
    pub span: Span,
}

/// One paramter as declared.
#[derive(Clone, Debug)]
pub struct Param {
    pub id: NodeId,
    pub name: Ident,
    pub ty: TypeExpr,
    pub kind: ParamKind,
    pub span: Span,
}

/// Which of the three paramter forms of 12.1 this one is.
#[derive(Clone, Debug)]
pub enum ParamKind {
    Required,
    Default(Expr),
    Variadic,
}

/// `struct User { ... }` (grammar 12.2).
#[derive(Clone, Debug)]
pub struct StructDecl {
    pub id: NodeId,
    pub visibility: Visibility,
    pub name: Ident,
    pub generics: Vec<GenericParam>,
    pub members: Vec<StructMember>,
    pub span: Span,
}

impl StructDecl {
    pub fn fields(&self) -> impl Iterator<Item = &FieldDecl> {
        self.members.iter().filter_map(|member| match member {
            StructMember::Field(field) => Some(field),
            StructMember::Method(_) => None,
        })
    }

    pub fn methods(&self) -> impl Iterator<Item = &FunctionDecl> {
        self.members.iter().filter_map(|member| match member {
            StructMember::Method(method) => Some(method),
            StructMember::Field(_) => None,
        })
    }
}

/// One thing inside a struct body.
#[derive(Clone, Debug)]
pub enum StructMember {
    Field(FieldDecl),
    Method(FunctionDecl),
}

impl StructMember {
    pub fn span(&self) -> Span {
        match self {
            StructMember::Field(field) => field.span,
            StructMember::Method(method) => method.span,
        }
    }

    pub fn name(&self) -> &Ident {
        match self {
            StructMember::Field(field) => &field.name,
            StructMember::Method(method) => &method.name,
        }
    }
}

/// `name: T` inside a struct body.
#[derive(Clone, Debug)]
pub struct FieldDecl {
    pub id: NodeId,
    pub visibility: Visibility,
    pub name: Ident,
    pub ty: TypeExpr,
    pub span: Span,
}

/// `enum Color { ... }` (grammar 12.3).
#[derive(Clone, Debug)]
pub struct EnumDecl {
    pub id: NodeId,
    pub visibility: Visibility,
    pub name: Ident,
    pub generics: Vec<GenericParam>,
    pub members: Vec<EnumMember>,
    pub span: Span,
}

impl EnumDecl {
    pub fn variants(&self) -> impl Iterator<Item = &VariantDecl> {
        self.members.iter().filter_map(|member| match member {
            EnumMember::Variant(variant) => Some(variant),
            EnumMember::Method(_) => None,
        })
    }

    pub fn methods(&self) -> impl Iterator<Item = &FunctionDecl> {
        self.members.iter().filter_map(|member| match member {
            EnumMember::Method(method) => Some(method),
            EnumMember::Variant(_) => None,
        })
    }
}

#[derive(Clone, Debug)]
pub enum EnumMember {
    Variant(VariantDecl),
    Method(FunctionDecl),
}

impl EnumMember {
    pub fn span(&self) -> Span {
        match self {
            EnumMember::Variant(variant) => variant.span,
            EnumMember::Method(method) => method.span,
        }
    }

    pub fn name(&self) -> &Ident {
        match self {
            EnumMember::Variant(variant) => &variant.name,
            EnumMember::Method(method) => &method.name,
        }
    }
}

/// `Red` or `Ok(T)` (grammar 12.3).
#[derive(Clone, Debug)]
pub struct VariantDecl {
    pub id: NodeId,
    pub visibility: Visibility,
    pub name: Ident,
    pub payload: Vec<TypeExpr>,
    pub span: Span,
}

/// `interface Printable { ... }` (grammar 12.4).
#[derive(Clone, Debug)]
pub struct InterfaceDecl {
    pub id: NodeId,
    pub visibility: Visibility,
    pub name: Ident,
    pub generics: Vec<GenericParam>,
    pub methods: Vec<InterfaceMethod>,
    pub span: Span,
}

/// `let p = e` (grammar 12.5).
#[derive(Clone, Debug)]
pub struct LetDecl {
    pub id: NodeId,
    pub pattern: IrrefutablePattern,
    pub ty: Option<TypeExpr>,
    pub value: Expr,
    pub span: Span,
}

/// `const MAX: int = 250` (grammar 12.5).
#[derive(Clone, Debug)]
pub struct ConstDecl {
    pub id: NodeId,
    pub visibility: Visibility,
    pub pattern: IrrefutablePattern,
    pub ty: Option<TypeExpr>,
    pub value: Expr,
    pub span: Span,
}

// ===== kieu, dung nhu

/// `Type` or `module.Type` (grammar 11.2).
#[derive(Clone, Debug)]
pub struct TypePath {
    pub module: Option<Ident>,
    pub name: Ident,
    pub span: Span,
}

/// A type the way source spells it.
#[derive(Clone, Debug)]
pub struct TypeExpr {
    pub id: NodeId,
    pub kind: TypeExprKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum TypeExprKind {
    Path {
        path: TypePath,
        args: Vec<TypeExpr>,
    },
    Array(Box<TypeExpr>),
    Map {
        key: Box<TypeExpr>,
        value: Box<TypeExpr>,
    },
    Set(Box<TypeExpr>),
    Tuple(Vec<TypeExpr>),
    Function(FunctionTypeExpr),
    Optional(Box<TypeExpr>),
    Failable(Box<TypeExpr>),
    Group(Box<TypeExpr>),
}

/// `fn(int, ...string): bool` (grammar 11).
#[derive(Clone, Debug)]
pub struct FunctionTypeExpr {
    pub params: Vec<TypeExpr>,
    pub variadic: Option<Box<TypeExpr>>,
    pub return_type: Option<Box<TypeExpr>>,
    pub span: Span,
}

// ===== statement (13) =====

/// `{ ... }`.
#[derive(Clone, Debug)]
pub struct Block {
    pub id: NodeId,
    pub statements: Vec<Stmt>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Stmt {
    pub id: NodeId,
    pub kind: StmtKind,
    pub span: Span,
}

/// All the statement shapes of grammar 13.
#[derive(Clone, Debug)]
pub enum StmtKind {
    Let(LetDecl),
    Const(ConstDecl),
    Assign(AssignStmt),
    Expr(Expr),
    If(IfStmt),
    While(WhileStmt),
    For(ForStmt),
    Match(MatchStmt),
    Return(Option<Expr>),
    Fail(Expr),
    Break,
    Continue,
    Block(Block),
}

/// `target op= value` (grammar 13.1).
#[derive(Clone, Debug)]
pub struct AssignStmt {
    pub target: Expr,
    pub op: AssignOp,
    pub value: Expr,
    pub span: Span,
}

/// Sau toan tu gan gop. Chi sau, khong hon.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AssignOp {
    Assign,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

impl AssignOp {
    /// Binary operator that a compound assign turns into. None for plain =.
    pub fn binary_op(self) -> Option<BinaryOp> {
        match self {
            AssignOp::Assign => None,
            AssignOp::Add => Some(BinaryOp::Add),
            AssignOp::Sub => Some(BinaryOp::Sub),
            AssignOp::Mul => Some(BinaryOp::Mul),
            AssignOp::Div => Some(BinaryOp::Div),
            AssignOp::Rem => Some(BinaryOp::Rem),
        }
    }
}

#[derive(Clone, Debug)]
pub struct IfStmt {
    pub condition: Expr,
    pub then_block: Block,
    pub else_branch: Option<ElseBranch>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum ElseBranch {
    If(Box<IfStmt>),
    Block(Block),
}

impl ElseBranch {
    pub fn span(&self) -> Span {
        match self {
            ElseBranch::If(stmt) => stmt.span,
            ElseBranch::Block(block) => block.span,
        }
    }
}

#[derive(Clone, Debug)]
pub struct WhileStmt {
    pub condition: Expr,
    pub body: Block,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct ForStmt {
    pub pattern: IrrefutablePattern,
    pub iterable: Expr,
    pub body: Block,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct MatchStmt {
    pub scrutinee: Expr,
    pub arms: Vec<MatchArm>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct MatchArm {
    pub id: NodeId,
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub body: MatchArmBody,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum MatchArmBody {
    Block(Block),
    Stmt(Box<Stmt>),
}

impl MatchArmBody {
    pub fn span(&self) -> Span {
        match self {
            MatchArmBody::Block(block) => block.span,
            MatchArmBody::Stmt(stmt) => stmt.span,
        }
    }
}

// ===== bieu thuc (14) =====

#[derive(Clone, Debug)]
pub struct Expr {
    pub id: NodeId,
    pub kind: ExprKind,
    pub span: Span,
}

impl Expr {
    /// True for the shapes 13.1 lets you assign into: ident, `this`, and any
    /// chain of .field / [index] on top of one of those.
    pub fn is_lvalue(&self) -> bool {
        match &self.kind {
            ExprKind::Ident(_) | ExprKind::This => true,
            ExprKind::Field { base, .. } | ExprKind::Index { base, .. } => base.is_lvalue(),
            _ => false,
        }
    }

    /// True if the postfix chain has at least one call in it. 13.2.1 wants
    /// that before an expression may stand alone as a statement.
    pub fn contains_call(&self) -> bool {
        match &self.kind {
            ExprKind::Call { .. } => true,
            ExprKind::Field { base, .. }
            | ExprKind::TupleField { base, .. }
            | ExprKind::Index { base, .. }
            | ExprKind::TypeArgs { base, .. } => base.contains_call(),
            ExprKind::ErrorPropagate(inner) | ExprKind::NullPropagate(inner) => {
                inner.contains_call()
            }
            ExprKind::Catch { operand, .. } => operand.contains_call(),
            _ => false,
        }
    }

    /// True for path shapes where a `{` after them may open a struct
    /// literal: ident, or module.Name, each maybe with one ::<T> behind.
    pub fn is_struct_literal_path(&self) -> bool {
        match &self.kind {
            ExprKind::Ident(_) => true,
            ExprKind::Field { base, .. } => matches!(base.kind, ExprKind::Ident(_)),
            ExprKind::TypeArgs { base, .. } => match &base.kind {
                ExprKind::Ident(_) => true,
                ExprKind::Field { base, .. } => matches!(base.kind, ExprKind::Ident(_)),
                _ => false,
            },
            _ => false,
        }
    }
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    // ---- so cap (14) ----
    Int(u64),
    Float(f64),
    Char(char),
    Bool(bool),
    Str(StringLit),
    Null,
    This,
    Ident(Ident),
    Array(Vec<Expr>),
    Map(Vec<MapEntry>),
    Set(Vec<Expr>),
    Tuple(Vec<Expr>),
    Group(Box<Expr>),
    Closure(ClosureExpr),

    // ---- toan tu ----
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Range {
        start: Box<Expr>,
        end: Box<Expr>,
        inclusive: bool,
    },
    Catch {
        operand: Box<Expr>,
        handler: CatchHandler,
    },

    // ---- hau to (14.17), lam tu trai sang phai, dung thu tu ----
    Field {
        base: Box<Expr>,
        name: Ident,
    },
    TupleField {
        base: Box<Expr>,
        index: u32,
        index_span: Span,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Argument>,
    },
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
    },
    NullPropagate(Box<Expr>),
    ErrorPropagate(Box<Expr>),
    TypeArgs {
        base: Box<Expr>,
        args: Vec<TypeExpr>,
    },
    StructLit(StructLit),
}

/// A string literal cut up into plain text and interpolation pieces.
#[derive(Clone, Debug)]
pub struct StringLit {
    pub parts: Vec<StringPart>,
    pub span: Span,
}

impl StringLit {
    /// Ca chuoi duoi dang text, khi no khong co noi suy nao.
    pub fn as_plain(&self) -> Option<String> {
        let mut out = String::new();
        for part in &self.parts {
            match part {
                StringPart::Text { value, .. } => out.push_str(value),
                StringPart::Interp(_) => return None,
            }
        }
        Some(out)
    }
}

#[derive(Clone, Debug)]
pub enum StringPart {
    Text { value: String, span: Span },
    Interp(Box<Expr>),
}

impl StringPart {
    pub fn span(&self) -> Span {
        match self {
            StringPart::Text { span, .. } => *span,
            StringPart::Interp(expr) => expr.span,
        }
    }
}

#[derive(Clone, Debug)]
pub struct MapEntry {
    pub key: Expr,
    pub value: Expr,
    pub span: Span,
}

/// `User { name: "x", age: 18 }` (grammar 14.13).
#[derive(Clone, Debug)]
pub struct StructLit {
    pub path: TypePath,
    pub type_args: Vec<TypeExpr>,
    pub fields: Vec<FieldInit>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct FieldInit {
    pub name: Ident,
    pub value: Expr,
    pub span: Span,
}

/// One argument of a call.
#[derive(Clone, Debug)]
pub struct Argument {
    pub name: Option<Ident>,
    pub value: Expr,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct ClosureExpr {
    pub id: NodeId,
    pub params: Vec<Param>,
    pub return_type: Option<TypeExpr>,
    pub body: Block,
    pub span: Span,
}

/// Ba dang cua `catch`, 14.1.
#[derive(Clone, Debug)]
pub enum CatchHandler {
    Discard(Block),
    Bind { name: Ident, block: Block },
    Value(Box<Expr>),
}

impl CatchHandler {
    pub fn span(&self) -> Span {
        match self {
            CatchHandler::Discard(block) => block.span,
            CatchHandler::Bind { name, block } => name.span.to(block.span),
            CatchHandler::Value(expr) => expr.span,
        }
    }
}

/// Khong co `+` mot ngoi, cung khong co `~`. 14.12.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UnaryOp {
    Not,
    Neg,
}

/// Binary operators, xep theo dung thu tu uu tien cua precedence.md.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BinaryOp {
    // nhan chia
    Mul,
    Div,
    Rem,
    // cong tru
    Add,
    Sub,
    // dich bit
    Shl,
    Shr,
    // theo bit, tat ca deu bam chat hon so sanh
    BitAnd,
    BitXor,
    BitOr,
    // so sanh, khong ket hop
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    // logic, co ngat som
    And,
    Or,
}

impl BinaryOp {
    pub fn is_comparison(self) -> bool {
        use BinaryOp::*;
        matches!(self, Eq | Ne | Lt | Gt | Le | Ge)
    }

    pub fn is_logical(self) -> bool {
        matches!(self, BinaryOp::And | BinaryOp::Or)
    }

    pub fn is_bitwise(self) -> bool {
        use BinaryOp::*;
        matches!(self, BitAnd | BitXor | BitOr | Shl | Shr)
    }

    pub fn is_arithmetic(self) -> bool {
        use BinaryOp::*;
        matches!(self, Add | Sub | Mul | Div | Rem)
    }

    /// How the operator is spelled, for error messages.
    pub fn spelling(self) -> &'static str {
        use BinaryOp::*;
        match self {
            Mul => "*",
            Div => "/",
            Rem => "%",
            Add => "+",
            Sub => "-",
            Shl => "<<",
            Shr => ">>",
            BitAnd => "&",
            BitXor => "^",
            BitOr => "|",
            Eq => "==",
            Ne => "!=",
            Lt => "<",
            Gt => ">",
            Le => "<=",
            Ge => ">=",
            And => "&&",
            Or => "||",
        }
    }
}

// `defer f()`. Lexer nhan tu khoa nay roi, o day cung de san cho, con
// parser thi van bao "chua lam". Xem TODO.txt.
#[derive(Clone, Debug)]
pub struct DeferStmt {
    pub call: Expr,
    pub span: Span,
}

// ===== pattern (15) =====

#[derive(Clone, Debug)]
pub struct Pattern {
    pub id: NodeId,
    pub kind: PatternKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum PatternKind {
    Wildcard,
    Null,
    Bool(bool),
    Int {
        magnitude: u64,
        negative: bool,
    },
    Char(char),
    Str(String),
    Range {
        start: RangeEndpoint,
        end: RangeEndpoint,
        inclusive: bool,
    },
    Binding(Ident),
    Variant {
        enum_name: Ident,
        variant: Ident,
        payload: Option<Vec<Pattern>>,
    },
    Struct {
        name: Ident,
        fields: Vec<FieldPattern>,
        rest: bool,
    },
    Tuple(Vec<Pattern>),
    Or(Vec<Pattern>),
}

#[derive(Clone, Copy, Debug)]
pub enum RangeEndpoint {
    Int { magnitude: u64, negative: bool },
    Char(char),
}

/// `name: pattern`, hoac viet tat `name` thi buoc luon bien cung ten voi
/// field vao gia tri cua field. 15.6.
#[derive(Clone, Debug)]
pub struct FieldPattern {
    pub name: Ident,
    pub pattern: Option<Pattern>,
    pub span: Span,
}

/// Nhung pattern dung duoc trong let, const va for. 15.8.
#[derive(Clone, Debug)]
pub struct IrrefutablePattern {
    pub id: NodeId,
    pub kind: IrrefutablePatternKind,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum IrrefutablePatternKind {
    Binding(Ident),
    Wildcard,
    Tuple(Vec<IrrefutablePattern>),
}

impl IrrefutablePattern {
    /// Every name this pattern binds, theo thu tu trong source.
    pub fn bindings(&self) -> Vec<&Ident> {
        let mut out = Vec::new();
        self.collect_bindings(&mut out);
        out
    }

    fn collect_bindings<'a>(&'a self, out: &mut Vec<&'a Ident>) {
        match &self.kind {
            IrrefutablePatternKind::Binding(name) => out.push(name),
            IrrefutablePatternKind::Wildcard => {}
            IrrefutablePatternKind::Tuple(elements) => {
                for element in elements {
                    element.collect_bindings(out);
                }
            }
        }
    }
}

// ===== test =====
//
// khong test parser o day, chi test may ham nho tu tay t viet trong file
// nay. Ham nao chi la field thi khong test.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::FileId;

    fn sp() -> Span {
        Span::new(FileId(0), 0, 1, 1, 1)
    }

    fn ident(name: &str) -> Ident {
        Ident::new(name.to_string(), sp())
    }

    fn ty(name: &str) -> TypeExpr {
        TypeExpr {
            id: NodeId::NONE,
            kind: TypeExprKind::Path {
                path: TypePath {
                    module: None,
                    name: ident(name),
                    span: sp(),
                },
                args: Vec::new(),
            },
            span: sp(),
        }
    }

    fn field(name: &str) -> StructMember {
        StructMember::Field(FieldDecl {
            id: NodeId::NONE,
            visibility: Visibility::implicit_private(),
            name: ident(name),
            ty: ty("int"),
            span: sp(),
        })
    }

    fn method(name: &str) -> StructMember {
        StructMember::Method(FunctionDecl {
            id: NodeId::NONE,
            visibility: Visibility::implicit_private(),
            name: ident(name),
            generics: Vec::new(),
            params: Vec::new(),
            return_type: None,
            body: Block {
                id: NodeId::NONE,
                statements: Vec::new(),
                span: sp(),
            },
            span: sp(),
        })
    }

    #[test]
    fn allocator_dem_tu_khong_va_khong_lap() {
        let mut ids = NodeIdAllocator::new();
        assert_eq!(ids.count(), 0);
        let a = ids.allocate();
        let b = ids.allocate();
        assert_eq!(a, NodeId(0));
        assert_eq!(b, NodeId(1));
        assert!(a != b);
        assert_eq!(ids.count(), 2);
        // NONE khong bao gio dung voi cai nao that
        assert!(a != NodeId::NONE);
    }

    #[test]
    fn khong_ghi_pub_thi_la_private() {
        let v = Visibility::default();
        assert!(!v.is_public());
        assert_eq!(v.kind, VisibilityKind::Private);
        // span None nghia la trong source khong co chu nao ca
        assert!(v.span.is_none());
    }

    #[test]
    fn import_khong_alias_thi_lay_doan_cuoi() {
        let im = Import {
            id: NodeId::NONE,
            path: vec![ident("net"), ident("http")],
            alias: None,
            span: sp(),
        };
        assert_eq!(im.bound_name().name, "http");
    }

    #[test]
    fn import_co_alias_thi_lay_alias() {
        let im = Import {
            id: NodeId::NONE,
            path: vec![ident("net"), ident("http")],
            alias: Some(ident("h")),
            span: sp(),
        };
        assert_eq!(im.bound_name().name, "h");
    }

    #[test]
    fn field_voi_method_nam_chung_mot_vec_nhung_loc_ra_duoc() {
        // luat 2 o dau file: giu nguyen thu tu source, nen o day co y xen ke
        let decl = StructDecl {
            id: NodeId::NONE,
            visibility: Visibility::implicit_private(),
            name: ident("User"),
            generics: Vec::new(),
            members: vec![field("id"), method("greet"), field("age")],
            span: sp(),
        };
        let names: Vec<String> = decl.fields().map(|f| f.name.name.clone()).collect();
        assert_eq!(names, vec!["id".to_string(), "age".to_string()]);
        assert_eq!(decl.methods().count(), 1);
        assert_eq!(decl.methods().next().unwrap().name.name, "greet");
    }

    #[test]
    fn implements_khong_dat_ten_moi() {
        let decl = Declaration::Implements(ImplementsDecl {
            id: NodeId::NONE,
            subject: ident("User"),
            interfaces: Vec::new(),
            span: sp(),
        });
        assert!(decl.name().is_none());
    }

    #[test]
    fn pattern_tra_ten_theo_thu_tu_source() {
        let p = IrrefutablePattern {
            id: NodeId::NONE,
            kind: IrrefutablePatternKind::Tuple(vec![
                IrrefutablePattern {
                    id: NodeId::NONE,
                    kind: IrrefutablePatternKind::Binding(ident("a")),
                    span: sp(),
                },
                IrrefutablePattern {
                    id: NodeId::NONE,
                    kind: IrrefutablePatternKind::Wildcard,
                    span: sp(),
                },
                IrrefutablePattern {
                    id: NodeId::NONE,
                    kind: IrrefutablePatternKind::Binding(ident("b")),
                    span: sp(),
                },
            ]),
            span: sp(),
        };
        let got: Vec<String> = p.bindings().iter().map(|n| n.name.clone()).collect();
        assert_eq!(got, vec!["a".to_string(), "b".to_string()]);
    }
}
