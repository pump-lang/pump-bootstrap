// kieu da giai xong, kem may bang khai bao ma resolver dien vao va ai
// dung sau cung doc.
//
// kieu duoc intern. TypeId chi la mot so nho Copy duoc, hai kieu giong nhau
// ve cau truc thi luon cung mot id, nen so sanh hai kieu chi la `==`. Rieng
// bien suy dien la ngoai le, chung co y khac nhau va giai qua substitution.
//
// TypeContext giu: bo intern, bo substitution, bang khai bao kieu tra theo
// cai nay defid, va bang
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
    // cai nay kieu co ban
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
