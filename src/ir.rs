// IR o giua. lower.rs ghi vao, clif.rs doc ra, va file nay la thu duy nhat
// hai ben do dung chung.
//
// SSA don gian, co y nan giong Cranelift de backend chi con la mot ban dich
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

impl Program {
    pub fn new() -> Program {
        Program::default()
    }

    pub fn function(&self, func: FuncRef) -> &Function {
        &self.functions[func.index()]
    }

    pub fn function_mut(&mut self, func: FuncRef) -> &mut Function {
        &mut self.functions[func.index()]
    }

    pub fn add_function(&mut self, function: Function) -> FuncRef {
        let handle = FuncRef(self.functions.len() as u32);
        self.functions.push(function);
        handle
    }

    pub fn signature(&self, sig: SigRef) -> &Signature {
        &self.signatures[sig.index()]
    }

    /// Intern one physical signature.
    pub fn add_signature(&mut self, signature: Signature) -> SigRef {
        if let Some(index) = self.signatures.iter().position(|s| *s == signature) {
            return SigRef(index as u32);
        }
        let handle = SigRef(self.signatures.len() as u32);
        self.signatures.push(signature);
        handle
    }

    /// Intern one string literal.
    pub fn add_string(&mut self, text: impl Into<String>) -> StringRef {
        let text = text.into();
        if let Some(index) = self.strings.iter().position(|s| *s == text) {
            return StringRef(index as u32);
        }
        let handle = StringRef(self.strings.len() as u32);
        self.strings.push(text);
        handle
    }

    pub fn add_global(&mut self, global: Global) -> GlobalRef {
        let handle = GlobalRef(self.globals.len() as u32);
        self.globals.push(global);
        handle
    }

    pub fn add_itable(&mut self, itable: Itable) -> ItableRef {
        let handle = ItableRef(self.itables.len() as u32);
        self.itables.push(itable);
        handle
    }

    /// May o se di vao pump_global_roots, dung thu tu.
    pub fn root_globals(&self) -> impl Iterator<Item = GlobalRef> + '_ {
        self.globals
            .iter()
            .enumerate()
            .filter(|(_, global)| global.ty.is_pointer())
            .map(|(index, _)| GlobalRef(index as u32))
    }
}

/// Chu ky vat ly cua ham: cai ma may nhin thay, sau khi gia tri mac dinh
/// da duoc dung ra va receiver da duoc gan len dau.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Signature {
    pub params: Vec<IrType>,
    pub ret: Option<IrType>,
}

impl Signature {
    pub fn new(params: Vec<IrType>, ret: Option<IrType>) -> Signature {
        Signature { params, ret }
    }
}

/// Cho chua cua mot hang muc module.
#[derive(Clone, Debug)]
pub struct Global {
    pub name: String,
    pub ty: IrType,
    pub span: Span,
}

/// Mot itable, sinh ra duoi dang du lieu tinh chi doc.
#[derive(Clone, Debug)]
pub struct Itable {
    pub name: String,
    pub interface_id: u64,
    pub concrete_type: TypeIdx,
    pub methods: Vec<FuncRef>,
}

/// Mot object tinh dung mot ban cho variant enum khong co payload.
#[derive(Clone, Debug)]
pub struct EnumSingleton {
    pub name: String,
    pub type_id: TypeIdx,
    pub tag: u32,
}

// ===== Function =====

/// One function after compiling.
#[derive(Clone, Debug)]
pub struct Function {
    pub name: String,
    pub signature: Signature,
    pub failable: bool,
    pub exported: bool,
    pub blocks: Vec<Block>,
    pub instructions: Vec<Inst>,
    pub values: Vec<ValueData>,
    pub slots: Vec<StackSlot>,
    pub entry: BlockRef,
    pub source_return_type: TypeId,
    pub span: Span,
}

impl Function {
    /// Make a function with an empty entry block, tham so khop signature.
    pub fn new(name: impl Into<String>, signature: Signature, span: Span) -> Function {
        let mut function = Function {
            name: name.into(),
            signature: signature.clone(),
            failable: false,
            exported: false,
            blocks: Vec::new(),
            instructions: Vec::new(),
            values: Vec::new(),
            slots: Vec::new(),
            entry: BlockRef(0),
            source_return_type: TypeId::VOID,
            span,
        };
        let entry = function.new_block(span);
        debug_assert_eq!(entry, BlockRef(0));
        for &param in &signature.params {
            function.add_block_param(entry, param);
        }
        function
    }

    pub fn block(&self, block: BlockRef) -> &Block {
        &self.blocks[block.index()]
    }

    pub fn block_mut(&mut self, block: BlockRef) -> &mut Block {
        &mut self.blocks[block.index()]
    }

    pub fn inst(&self, inst: InstRef) -> &Inst {
        &self.instructions[inst.index()]
    }

    pub fn value(&self, value: Value) -> &ValueData {
        &self.values[value.index()]
    }

    pub fn value_type(&self, value: Value) -> IrType {
        self.values[value.index()].ty
    }

    /// Tham so cua ham, cung chinh la tham so cua block dau tien.
    pub fn params(&self) -> &[Value] {
        &self.blocks[self.entry.index()].params
    }

    pub fn new_block(&mut self, span: Span) -> BlockRef {
        let handle = BlockRef(self.blocks.len() as u32);
        self.blocks.push(Block {
            params: Vec::new(),
            instructions: Vec::new(),
            terminator: Terminator::Unreachable,
            span,
        });
        handle
    }

    /// Them mot tham so vao block, tra ve Value ma no dinh nghia.
    pub fn add_block_param(&mut self, block: BlockRef, ty: IrType) -> Value {
        let index = self.blocks[block.index()].params.len() as u32;
        let value = self.new_value(ty, ValueDef::BlockParam { block, index });
        self.blocks[block.index()].params.push(value);
        value
    }

    /// Declare a stack slot.
    pub fn add_slot(&mut self, size: u32, align: u32, span: Span) -> SlotRef {
        let handle = SlotRef(self.slots.len() as u32);
        self.slots.push(StackSlot { size, align, span });
        handle
    }

    /// Declare a stack slot big enough for one value of ty.
    pub fn add_slot_for(&mut self, ty: IrType, span: Span) -> SlotRef {
        self.add_slot(ty.size(), ty.align(), span)
    }

    /// Them mot lenh vao block, tra ve Value neu lenh do co dinh nghia.
    pub fn push(&mut self, block: BlockRef, kind: InstKind, span: Span) -> Option<Value> {
        let result_type = kind.result_type(self);
        let inst = InstRef(self.instructions.len() as u32);
        let result = result_type.map(|ty| self.new_value(ty, ValueDef::Inst(inst)));
        self.instructions.push(Inst { kind, result, span });
        self.blocks[block.index()].instructions.push(inst);
        result
    }

    /// Them mot lenh ma chac chan la co dinh nghia gia tri.
    pub fn push_value(&mut self, block: BlockRef, kind: InstKind, span: Span) -> Value {
        self.push(block, kind, span)
            .expect("this instruction defines a value")
    }

    pub fn set_terminator(&mut self, block: BlockRef, terminator: Terminator) {
        self.blocks[block.index()].terminator = terminator;
    }

    fn new_value(&mut self, ty: IrType, def: ValueDef) -> Value {
        let handle = Value(self.values.len() as u32);
        self.values.push(ValueData { ty, def });
        handle
    }
}
