// kieu da giai xong, kem may bang khai bao ma resolver dien vao va ai
// dung sau cung doc.
//
// kieu duoc intern. TypeId chi la mot so nho Copy duoc, hai kieu giong nhau
// ve cau truc thi luon cung mot id, nen so sanh hai kieu chi la `==`. Rieng
// bien suy dien la ngoai le, chung co y khac nhau va giai qua substitution.
//
// TypeContext giu: bo intern, bo substitution, bang khai bao kieu tra theo
// DefId, va bang ham/method tra theo FuncId.
//
// obj -> type

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::rc::Rc;

use crate::ast::VisibilityKind;
use crate::token::Span;

/// Id of one module. Mot module la mot file, 10.2.7.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ModuleId(pub u32);

impl ModuleId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Id of one struct, enum or interface.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct DefId(pub u32);

impl DefId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Id of one function, method, or interface method signature.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct FuncId(pub u32);

impl FuncId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// An interned type. So sanh hai cai chi can `==`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct TypeId(pub u32);

impl TypeId {
    // kieu co ban chiem cac o co dinh, intern san boi TypeContext::new, the
    // nen goi ten chung bang hang duoc
    pub const ERROR: TypeId = TypeId(0);
    pub const VOID: TypeId = TypeId(1);
    pub const NEVER: TypeId = TypeId(2);
    pub const BOOL: TypeId = TypeId(3);
    pub const INT: TypeId = TypeId(4);
    pub const UINT: TypeId = TypeId(5);
    pub const FLOAT: TypeId = TypeId(6);
    pub const CHAR: TypeId = TypeId(7);
    pub const STRING: TypeId = TypeId(8);

    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// An inference variable. Checker de ra, unify giai.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct TypeVar(pub u32);

impl TypeVar {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// A bound generic parameter, e.g.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct GenericId {
    pub owner: GenericOwner,
    pub index: u32,
}

/// Who the generic paramter belongs to.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum GenericOwner {
    Type(DefId),
    Func(FuncId),
}

/// Shape of a type.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum TypeKind {
    Error,
    Void,
    Never,

    Bool,
    Int,
    Uint,
    Float,
    Char,
    String,

    Array(TypeId),
    Map { key: TypeId, value: TypeId },
    Set(TypeId),
    Tuple(Vec<TypeId>),

    Optional(TypeId),
    Failable(TypeId),

    Function(FnType),

    Named { def: DefId, args: Vec<TypeId> },

    Generic(GenericId),

    Var(TypeVar),

    UntypedInt,
    UntypedFloat,
}

/// Type of a function value. Khong co ten tham so, khong co default.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct FnType {
    pub params: Vec<TypeId>,
    pub variadic: Option<TypeId>,
    pub ret: TypeId,
    pub failable: bool,
}

// -- khai bao
