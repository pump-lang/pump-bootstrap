// IR o giua. lower.rs ghi vao, clif.rs doc ra, va file nay la thu duy nhat
// hai ben do dung chung.
//
// SSA don gian, ...
// chu khong phai mot cai compiler thu hai:
//
//  * Function la mot day Block, block nao cung ket thuc bang dung mot
//    Terminator,
//  * block nhan tham so, nhanh thi truyen doi so. Khong co phi node.
//  * mot lenh dinh nghia nhieu nhat mot Value, va Value chi duoc dinh nghia
//    dung mot lan,
//  * bien co the sua duoc thi nam trong o stack, lay dia chi bang SlotAddr
//    roi dung Load voi Store.
//
// Hai cai duoi ...
// co kich thuoc san.
//
// Den luc co IR thi check voi mono xong het roi: khong con generic, khong
// con suy dien, khong con doi so co ten hay mac dinh. Moi loi goi truyen dung
// nhung gi ben kia can, va moi offset da la mot con so.

use crate::token::Span;
use crate::types::TypeId;

pub use crate::abi::{IrType, RuntimeFn};

/// Index into `Program::functions`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct FuncRef(pub u32);

/// Index into `Program::signatures`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct SigRef(pub u32);

/// Index into `Program::strings`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct StringRef(pub u32);

/// Index into `Program::globals`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct GlobalRef(pub u32);

/// Index into `Program::itables`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ItableRef(pub u32);

/// A runtime type id, i.e.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct TypeIdx(pub u32);

/// Index into `Function::blocks`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct BlockRef(pub u32);

/// Index into `Function::instructions`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct InstRef(pub u32);

/// Index into `Function::values`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Value(pub u32);

/// Index into `Function::slots`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct SlotRef(pub u32);

macro_rules! index_type {
    ($($ty:ty),* $(,)?) => {
        $(impl $ty {
            pub fn index(self) -> usize {
                self.0 as usize
            }
        })*
    };
}

index_type!(
    FuncRef, SigRef, StringRef, GlobalRef, ItableRef, TypeIdx, BlockRef, InstRef, Value, SlotRef,
);

// ===== Program ===== ...

/// Ca mot lan bien dich: het ham, het static, het descriptor.
#[derive(Clone, Debug, Default)]
pub struct Program {
    pub functions: Vec<Function>,
    pub signatures: Vec<Signature>,
    pub strings: Vec<String>,
    pub globals: Vec<Global>,
    pub itables: Vec<Itable>,
    pub type_descriptors: Vec<crate::abi::TypeDescriptor>,
    pub enum_singletons: Vec<EnumSingleton>,
    pub entry: Option<FuncRef>,
    pub module_init: Option<FuncRef>,
}
