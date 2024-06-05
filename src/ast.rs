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
