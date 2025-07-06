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
// Hai cai duoi anh thang sang Cranelift, no cung co tham so block va o stack
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

// ===== Program =====

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

/// Mot block: tham so, mot than thang tuot, va mot terminator.
#[derive(Clone, Debug)]
pub struct Block {
    pub params: Vec<Value>,
    pub instructions: Vec<InstRef>,
    pub terminator: Terminator,
    pub span: Span,
}

/// One stack slot.
#[derive(Clone, Debug)]
pub struct StackSlot {
    pub size: u32,
    pub align: u32,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct ValueData {
    pub ty: IrType,
    pub def: ValueDef,
}

#[derive(Clone, Copy, Debug)]
pub enum ValueDef {
    Inst(InstRef),
    BlockParam { block: BlockRef, index: u32 },
}

#[derive(Clone, Debug)]
pub struct Inst {
    pub kind: InstKind,
    pub result: Option<Value>,
    pub span: Span,
}

// ===== lenh =====

/// Every instruction the IR knows.
#[derive(Clone, Debug)]
pub enum InstKind {
    // ---- goi ham ----
    Call {
        func: FuncRef,
        args: Vec<Value>,
    },
    CallIndirect {
        callee: Value,
        sig: SigRef,
        args: Vec<Value>,
    },
    CallClosure {
        closure: Value,
        sig: SigRef,
        args: Vec<Value>,
    },
    CallInterface {
        object: Value,
        slot: u32,
        sig: SigRef,
        args: Vec<Value>,
    },
    CallRuntime {
        entry: RuntimeFn,
        args: Vec<Value>,
    },

    // ---- bo nho ----
    SlotAddr(SlotRef),
    GlobalAddr(GlobalRef),
    Load {
        ptr: Value,
        offset: i32,
        ty: IrType,
    },
    Store {
        ptr: Value,
        offset: i32,
        value: Value,
    },
    PtrAdd {
        ptr: Value,
        index: Value,
        stride: u32,
    },
    PtrOffset {
        ptr: Value,
        offset: i64,
    },

    // ---- may cai chan truoc ----
    BoundsCheck {
        index: Value,
        length: Value,
    },
    NullCheck {
        value: Value,
    },
    DivisorCheck {
        divisor: Value,
    },
    ShiftCountCheck {
        count: Value,
    },

    // ---- hang so ----
    ConstInt(i64),
    ConstFloat(f64),
    ConstBool(bool),
    ConstChar(u32),
    ConstNull,
    ConstString(StringRef),
    ConstFuncAddr(FuncRef),
    ConstItable(ItableRef),
    ConstEnumSingleton {
        type_id: TypeIdx,
        tag: u32,
    },

    // ---- o loi dang treo ----
    ErrorPending,
    ErrorTake,
    ErrorSet {
        error: Value,
    },

    // ---- so hoc va logic ----
    Binary {
        op: BinaryOp,
        lhs: Value,
        rhs: Value,
    },
    Unary {
        op: UnaryOp,
        value: Value,
    },
    Compare {
        op: CompareOp,
        lhs: Value,
        rhs: Value,
    },
    Convert {
        op: ConvertOp,
        value: Value,
    },
    Select {
        cond: Value,
        then_value: Value,
        else_value: Value,
    },

    // ---- cap phat ----
    Alloc {
        type_id: TypeIdx,
        size: u64,
    },
    AllocVariable {
        type_id: TypeIdx,
        size: Value,
    },
}

impl InstKind {
    /// The type of the value this instruction defines, or `None` when it
    /// defines nothing.
    pub fn result_type(&self, function: &Function) -> Option<IrType> {
        match self {
            InstKind::ConstInt(_) => Some(IrType::I64),
            InstKind::ConstFloat(_) => Some(IrType::F64),
            InstKind::ConstBool(_) => Some(IrType::I8),
            InstKind::ConstChar(_) => Some(IrType::I32),
            InstKind::ConstNull
            | InstKind::ConstString(_)
            | InstKind::ConstFuncAddr(_)
            | InstKind::ConstItable(_)
            | InstKind::ConstEnumSingleton { .. } => Some(IrType::Ptr),

            InstKind::Binary { lhs, .. } => Some(function.value_type(*lhs)),
            InstKind::Unary { value, .. } => Some(function.value_type(*value)),
            InstKind::Compare { .. } => Some(IrType::I8),
            InstKind::Convert { op, .. } => Some(op.result_type()),
            InstKind::Select { then_value, .. } => Some(function.value_type(*then_value)),

            InstKind::SlotAddr(_) | InstKind::GlobalAddr(_) => Some(IrType::Ptr),
            InstKind::Load { ty, .. } => Some(*ty),
            InstKind::Store { .. } => None,
            InstKind::PtrAdd { .. } | InstKind::PtrOffset { .. } => Some(IrType::Ptr),

            InstKind::Alloc { .. } | InstKind::AllocVariable { .. } => Some(IrType::Ptr),

            // A direct call's result type comes from the callee's signature,
            // which the caller of this method does not have. Lowering must
            // consult `Program::function` instead; `Function::push` is never
            // used for a call whose result is needed without going through
            // `push_call`.
            InstKind::Call { .. }
            | InstKind::CallIndirect { .. }
            | InstKind::CallClosure { .. }
            | InstKind::CallInterface { .. }
            | InstKind::CallRuntime { .. } => None,

            InstKind::ErrorPending => Some(IrType::I8),
            InstKind::ErrorTake => Some(IrType::Ptr),
            InstKind::ErrorSet { .. } => None,

            InstKind::BoundsCheck { .. }
            | InstKind::NullCheck { .. }
            | InstKind::DivisorCheck { .. }
            | InstKind::ShiftCountCheck { .. } => None,
        }
    }

    /// True for the instructions that can trigger a collection, and therefore
    /// invalidate any live interior pointer.
    pub fn may_allocate(&self) -> bool {
        matches!(
            self,
            InstKind::Alloc { .. }
                | InstKind::AllocVariable { .. }
                | InstKind::Call { .. }
                | InstKind::CallIndirect { .. }
                | InstKind::CallClosure { .. }
                | InstKind::CallInterface { .. }
                | InstKind::CallRuntime { .. }
        )
    }
}

impl Function {
    /// Appends a call whose result type the caller supplies, because a call's
    /// result type is not derivable from the instruction alone.
    pub fn push_call(
        &mut self,
        block: BlockRef,
        kind: InstKind,
        result: Option<IrType>,
        span: Span,
    ) -> Option<Value> {
        debug_assert!( matches!( kind, InstKind::Call { .. } | InstKind::CallIndirect { .. } | InstKind::CallClosure { .. } | InstKind::CallInterface { .. } | InstKind::CallRuntime { .. } ),
            "push_call takes a call instruction"
        );
        let inst = InstRef(self.instructions.len() as u32);
        let value = result.map(|ty| self.new_value(ty, ValueDef::Inst(inst)));
        self.instructions.push(Inst {
            kind,
            result: value,
            span,
        });
        self.blocks[block.index()].instructions.push(inst);
        value
    }
}

/// Binary operators.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BinaryOp {
    IAdd,
    ISub,
    IMul,
    SDiv,
    UDiv,
    SRem,
    URem,

    FAdd,
    FSub,
    FMul,
    FDiv,
    FRem,

    BitAnd,
    BitOr,
    BitXor,
    Shl,
    AShr,
    LShr,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UnaryOp {
    INeg,
    FNeg,
    Not,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CompareOp {
    IEq,
    INe,
    SLt,
    SGt,
    SLe,
    SGe,
    ULt,
    UGt,
    ULe,
    UGe,
    FEq,
    FNe,
    FLt,
    FGt,
    FLe,
    FGe,
}

/// Representation changes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConvertOp {
    FloatToInt,
    FloatToUint,
    IntToFloat,
    UintToFloat,
    CharToInt,
    SignExtend32To64,
    BoolToInt,
    IntToBool,
    IntToChar,
    BitcastFloatToInt,
    BitcastIntToFloat,
    IntToPtr,
    PtrToInt,
}

impl ConvertOp {
    pub fn result_type(self) -> IrType {
        match self {
            ConvertOp::FloatToInt
            | ConvertOp::FloatToUint
            | ConvertOp::CharToInt
            | ConvertOp::SignExtend32To64
            | ConvertOp::BoolToInt
            | ConvertOp::BitcastFloatToInt
            | ConvertOp::PtrToInt => IrType::I64,
            ConvertOp::IntToFloat | ConvertOp::UintToFloat | ConvertOp::BitcastIntToFloat => {
                IrType::F64
            }
            ConvertOp::IntToBool => IrType::I8,
            ConvertOp::IntToChar => IrType::I32,
            ConvertOp::IntToPtr => IrType::Ptr,
        }
    }
}

// ##### Terminators

/// How a block ends.
#[derive(Clone, Debug)]
pub enum Terminator {
    Jump {
        target: BlockRef,
        args: Vec<Value>,
    },
    Branch {
        cond: Value,
        then_block: BlockRef,
        then_args: Vec<Value>,
        else_block: BlockRef,
        else_args: Vec<Value>,
    },
    Switch {
        value: Value,
        cases: Vec<SwitchCase>,
        default: BlockRef,
    },
    Return {
        value: Option<Value>,
    },
    ReturnError {
        error: Value,
    },
    Unreachable,
}

#[derive(Clone, Copy, Debug)]
pub struct SwitchCase {
    pub value: i64,
    pub target: BlockRef,
}

impl Terminator {
    /// Every block this terminator can transfer control to.
    pub fn successors(&self) -> Vec<BlockRef> {
        match self {
            Terminator::Jump { target, .. } => vec![*target],
            Terminator::Branch {
                then_block,
                else_block,
                ..
            } => vec![*then_block, *else_block],
            Terminator::Switch { cases, default, .. } => {
                let mut targets: Vec<BlockRef> = cases.iter().map(|case| case.target).collect();
                targets.push(*default);
                targets
            }
            Terminator::Return { .. }
            | Terminator::ReturnError { .. }
            | Terminator::Unreachable => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span() -> Span {
        Span::synthetic()
    }

    #[test]
    fn the_entry_block_holds_the_function_parameters() {
        let signature = Signature::new(vec![IrType::I64, IrType::Ptr], Some(IrType::I64));
        let function = Function::new("pump$m$$f$", signature, span());
        assert_eq!(function.entry, BlockRef(0));
        assert_eq!(function.params().len(), 2);
        assert_eq!(function.value_type(function.params()[0]), IrType::I64);
        assert_eq!(function.value_type(function.params()[1]), IrType::Ptr);
    }

    #[test]
    fn instructions_define_values_of_the_right_type() {
        let mut function = Function::new("f", Signature::new(vec![], None), span());
        let entry = function.entry;
        let a = function.push_value(entry, InstKind::ConstInt(2), span());
        let b = function.push_value(entry, InstKind::ConstInt(40), span());
        let sum = function.push_value(
            entry,
            InstKind::Binary {
                op: BinaryOp::IAdd,
                lhs: a,
                rhs: b,
            },
            span(),
        );
        assert_eq!(function.value_type(sum), IrType::I64);

        let equal = function.push_value(
            entry,
            InstKind::Compare {
                op: CompareOp::IEq,
                lhs: a,
                rhs: sum,
            },
            span(),
        );
        assert_eq!(function.value_type(equal), IrType::I8);

        function.set_terminator(entry, Terminator::Return { value: None });
        assert!(function.block(entry).terminator.successors().is_empty());
    }

    #[test]
    fn a_store_defines_nothing() {
        let mut function = Function::new("f", Signature::new(vec![IrType::Ptr], None), span());
        let entry = function.entry;
        let ptr = function.params()[0];
        let value = function.push_value(entry, InstKind::ConstInt(7), span());
        let result = function.push(
            entry,
            InstKind::Store {
                ptr,
                offset: 16,
                value,
            },
            span(),
        );
        assert!(result.is_none());
    }

    #[test]
    fn signatures_and_strings_are_interned() {
        let mut program = Program::new();
        let a = program.add_signature(Signature::new(vec![IrType::I64], None));
        let b = program.add_signature(Signature::new(vec![IrType::I64], None));
        assert_eq!(a, b);
        assert_eq!(program.add_string("hi"), program.add_string("hi"));
        assert_eq!(program.strings.len(), 1);
    }
}
