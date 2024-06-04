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
