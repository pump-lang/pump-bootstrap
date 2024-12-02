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
    // runtime roi doi thanh `fail` cho tu te. Dat ten dai va rieng nhu the nay
    // de khong ai vo tinh dam vao, vi prelude thi ca chuong trinh nhin thay.
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

fn bundled_module(segments: &[String]) -> Option<&'static str> {
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

fn predeclared_value(name: &str) -> Option<Predeclared> {
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

fn conversion_target(name: &str) -> Option<TypeId> {
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
pub fn resolve(
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

// trang thai ben trong

#[derive(Debug, Default)]
struct ModuleInfo {
    path: Vec<String>,
    types: HashMap<String, DefId>,
    functions: HashMap<String, FuncId>,
    constants: HashMap<String, GlobalConstId>,
    imports: HashMap<String, ImportBinding>,
}

#[derive(Clone, Copy, Debug)]
struct ImportBinding {
    module: ModuleId,
    span: Span,
    used: bool,
}

#[derive(Debug)]
struct Frame {
    this_owner: Option<DefId>,
    blocks: Vec<HashMap<String, LocalId>>,
    closure: Option<NodeId>,
    captures: Vec<LocalId>,
    captures_this: bool,
    loop_depth: u32,
}

impl Frame {
    fn new(this_owner: Option<DefId>, closure: Option<NodeId>) -> Frame {
        Frame {
            this_owner,
            blocks: vec![HashMap::new()],
            closure,
            captures: Vec::new(),
            captures_this: false,
            loop_depth: 0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TypePosition {
    Return,
    Value,
}

struct Resolver<'a> {
    context: TypeContext,
    units: Vec<SourceUnit>,
    modules: Vec<ModuleInfo>,
    diagnostics: &'a mut Diagnostics,

    values: HashMap<NodeId, ValueBinding>,
    types: HashMap<NodeId, TypeId>,
    locals: Vec<LocalBinding>,
    declared_locals: HashMap<Span, LocalId>,
    closures: HashMap<NodeId, ClosureInfo>,
    functions: HashMap<FuncId, FuncLocation>,
    globals: Vec<GlobalConst>,
    implements: Vec<ImplementsAssertion>,
    pattern_defs: HashMap<NodeId, DefId>,
    const_init_order: Vec<GlobalConstId>,

    // cai nay hoi truoc chi la HashSet thoi. Luc them canh bao bien khong
    // dung thi lookup_value dang muon &mut cua no, ma ngoai kia dang giu
    // &self.locals, borrow checker keu ca buoi. Boc Rc<RefCell<>> vao la het
    // keu. Chac co cach khac gon hon nhung t tim khong ra.
    used_locals: Rc<RefCell<HashSet<LocalId>>>,
    const_dependencies: HashMap<GlobalConstId, Vec<GlobalConstId>>,
    current_const: Option<GlobalConstId>,
    pending_bindings: Vec<(String, Span)>,

    prelude: Option<Prelude>,
    root_module: ModuleId,
    entry: Option<FuncId>,

    module: ModuleId,
    generic_scopes: Vec<HashMap<String, GenericId>>,
    frames: Vec<Frame>,
}

impl<'a> Resolver<'a> {
    fn new(diagnostics: &'a mut Diagnostics) -> Resolver<'a> {
        Resolver {
            context: TypeContext::new(),
            units: Vec::new(),
            modules: Vec::new(),
            diagnostics,
            values: HashMap::new(),
            types: HashMap::new(),
            locals: Vec::new(),
            declared_locals: HashMap::new(),
            closures: HashMap::new(),
            functions: HashMap::new(),
            globals: Vec::new(),
            implements: Vec::new(),
            pattern_defs: HashMap::new(),
            const_init_order: Vec::new(),
            used_locals: Rc::new(RefCell::new(HashSet::new())),
            const_dependencies: HashMap::new(),
            current_const: None,
            pending_bindings: Vec::new(),
            prelude: None,
            root_module: ModuleId(0),
            entry: None,
            module: ModuleId(0),
            generic_scopes: Vec::new(),
            frames: Vec::new(),
        }
    }

    fn report(&mut self, error: CompileError) {
        self.diagnostics.push(error);
    }

    fn prelude(&self) -> Prelude {
        self.prelude
            .expect("the prelude is built before every pass")
    }

    fn finish(self) -> Resolution {
        let prelude = self.prelude();
        Resolution {
            context: self.context,
            units: self.units,
            root_module: self.root_module,
            entry: self.entry,
            values: self.values,
            types: self.types,
            locals: self.locals,
            const_init_order: self.const_init_order,
            globals: self.globals,
            declared_locals: self.declared_locals,
            closures: self.closures,
            functions: self.functions,
            implements: self.implements,
            pattern_defs: self.pattern_defs,
            prelude,
        }
    }

    // prelude

    fn build_prelude(&mut self) {
        let module = self.context.add_module(vec!["<builtin>".to_string()]);
        self.modules.push(ModuleInfo {
            path: vec!["<builtin>".to_string()],
            ..ModuleInfo::default()
        });
        self.units.push(SourceUnit {
            id: NodeId::NONE,
            module_path: vec!["<builtin>".to_string()],
            imports: Vec::new(),
            declarations: Vec::new(),
            span: Span::synthetic(),
        });

        let error = self.declare_prelude_interface(module, "Error", "message", TypeId::STRING);
        let stringable =
            self.declare_prelude_interface(module, "Stringable", "to_string", TypeId::STRING);
        let range = self.declare_prelude_range(module);

        let error_type = self.context.named(error, Vec::new());
        let range_type = self.context.named(range, Vec::new());
        self.prelude = Some(Prelude {
            module,
            error,
            stringable,
            range,
            error_type,
            range_type,
        });
    }

    fn declare_prelude_interface(
        &mut self,
        module: ModuleId,
        name: &str,
        method: &str,
        ret: TypeId,
    ) -> DefId {
        let def = self.context.next_def_id();
        let func = self.context.next_func_id();
        self.context.add_func(FuncDef {
            id: func,
            name: method.to_string(),
            module,
            owner: Some(def),
            visibility: VisibilityKind::Public,
            generics: Vec::new(),
            params: Vec::new(),
            ret,
            failable: false,
            has_receiver: true,
            has_body: false,
            span: Span::synthetic(),
        });
        self.context.add_def(TypeDef {
            id: def,
            name: name.to_string(),
            module,
            visibility: VisibilityKind::Public,
            generics: Vec::new(),
            obj_kind: TypeDefKind::Interface(InterfaceDef {
                methods: vec![func],
            }),
            span: Span::synthetic(),
        });
        def
    }

    fn declare_prelude_range(&mut self, module: ModuleId) -> DefId {
        let def = self.context.next_def_id();
        let field = |name: &str, ty: TypeId| FieldDef {
            name: name.to_string(),
            ty,
            visibility: VisibilityKind::Public,
            span: Span::synthetic(),
        };
        self.context.add_def(TypeDef {
            id: def,
            name: "Range".to_string(),
            module,
            visibility: VisibilityKind::Public,
            generics: Vec::new(),
            obj_kind: TypeDefKind::Struct(StructDef {
                fields: vec![
                    field("start", TypeId::INT),
                    field("end", TypeId::INT),
                    field("inclusive", TypeId::BOOL),
                ],
                methods: Vec::new(),
            }),
            span: Span::synthetic(),
        });
        def
    }

    // luot 1: do thi module

    fn load_graph(&mut self, entry: SourceUnit, project_root: &Path, session: &mut crate::Session) {
        let root_path = entry.module_path.clone();
        let root = self.context.add_module(root_path.clone());
        self.modules.push(ModuleInfo {
            path: root_path.clone(),
            ..ModuleInfo::default()
        });
        self.units.push(entry);
        self.root_module = root;

        let mut loaded: HashMap<String, ModuleId> = HashMap::new();
        loaded.insert(module_key(&root_path), root);
        let mut stack = vec![root_path];
        self.load_imports_of(root, project_root, session, &mut loaded, &mut stack);
    }

    fn load_imports_of(
        &mut self,
        module: ModuleId,
        project_root: &Path,
        session: &mut crate::Session,
        loaded: &mut HashMap<String, ModuleId>,
        stack: &mut Vec<Vec<String>>,
    ) {
        let imports = self.units[module.index()].imports.clone();
        for import in &imports {
            let segments: Vec<String> = import.path.iter().map(|part| part.name.clone()).collect();
            let key = module_key(&segments);
            let bound = import.bound_name().clone();

            if stack.iter().any(|entry| module_key(entry) == key) {
                self.report(
                    CompileError::at(
                        ErrorCode::CircularImport,
                        import.span,
                        format!("`{}` is already being imported", segments.join("\\")),
                    )
                    .with_note(format!(
                        "import cycle: {}",
                        stack
                            .iter()
                            .map(|entry| entry.join("\\"))
                            .collect::<Vec<_>>()
                            .join(" -> ")
                    )),
                );
                continue;
            }

            let target = match loaded.get(&key).copied() {
                Some(existing) => Some(existing),
                None => {
                    let loaded_module =
                        self.load_module(&segments, import.span, project_root, session);
                    if let Some(id) = loaded_module {
                        loaded.insert(key.clone(), id);
                        stack.push(segments.clone());
                        self.load_imports_of(id, project_root, session, loaded, stack);
                        stack.pop();
                    }
                    loaded_module
                }
            };

            let Some(target) = target else { continue };
            let info = &mut self.modules[module.index()];
            if let Some(previous) = info.imports.get(&bound.name).copied() {
                let error = CompileError::at(
                    ErrorCode::DuplicateImportBinding,
                    bound.span,
                    format!("`{}` is already bound by another import", bound.name),
                )
                .with_secondary(previous.span, "first bound here")
                .with_help("give one of them a different name with `as`");
                self.report(error);
                continue;
            }
            info.imports.insert(
                bound.name.clone(),
                ImportBinding {
                    module: target,
                    span: bound.span,
                    used: false,
                },
            );
        }
    }

    fn load_module(
        &mut self,
        segments: &[String],
        span: Span,
        project_root: &Path,
        session: &mut crate::Session,
    ) -> Option<ModuleId> {
        let mut path = PathBuf::from(project_root);
        for segment in segments {
            path.push(segment);
        }
        path.set_extension("pump");
        // println!("DBG: {:?} -> {:?}", segments, path);

        let file = if path.exists() {
            match session.load(&path) {
                Ok(file) => file,
                Err(error) => {
                    self.report(error);
                    return None;
                }
            }
        } else if let Some(source) = bundled_module(segments) {
            // file trong project trung ten thi luon thang, chi khi tim tren
            // dia that bai moi ngo toi ban dong goi san
            session.sources.add(&path, source.to_string())
        } else {
            self.report(
                CompileError::at(
                    ErrorCode::ModuleNotFound,
                    span,
                    format!("no source file for module `{}`", segments.join("\\")),
                )
                .with_help(format!("expected it at `{}`", path.display())),
            );
            return None;
        };
        let text = session
            .sources
            .get(file)
            .expect("the file was just loaded")
            .text
            .clone();

        let tokens = crate::lexer::tokenize(file, &text, self.diagnostics);
        let unit = crate::parser::parse(
            file,
            segments.to_vec(),
            &tokens,
            &mut session.node_ids,
            self.diagnostics,
        );

        let id = self.context.add_module(segments.to_vec());
        self.modules.push(ModuleInfo {
            path: segments.to_vec(),
            ..ModuleInfo::default()
        });
        self.units.push(unit);
        Some(id)
    }

    // luot 2: khai bao het cac muc o tren cung

    fn declare_items(&mut self) {
        for index in 1..self.units.len() {
            let module = ModuleId(index as u32);
            self.declare_module_items(module);
        }
    }

    fn declare_module_items(&mut self, module: ModuleId) {
        let declarations = self.units[module.index()].declarations.clone();
        for (position, declaration) in declarations.iter().enumerate() {
            match declaration {
                Declaration::Struct(decl) => {
                    let kind = TypeDefKind::Struct(StructDef {
                        fields: Vec::new(),
                        methods: Vec::new(),
                    });
                    self.declare_type(module, &decl.name, decl.visibility.kind, decl.span, kind);
                }
                Declaration::Enum(decl) => {
                    let kind = TypeDefKind::Enum(EnumDef {
                        variants: Vec::new(),
                        methods: Vec::new(),
                    });
                    self.declare_type(module, &decl.name, decl.visibility.kind, decl.span, kind);
                }
                Declaration::Interface(decl) => {
                    let kind = TypeDefKind::Interface(InterfaceDef {
                        methods: Vec::new(),
                    });
                    self.declare_type(module, &decl.name, decl.visibility.kind, decl.span, kind);
                }
                Declaration::Function(decl) => {
                    self.declare_function(module, position, decl);
                }
                Declaration::Const(decl) => {
                    self.declare_module_constant(module, position, decl);
                }
                Declaration::Implements(_) => {}
            }
        }
    }

    fn declare_type(
        &mut self,
        module: ModuleId,
        name: &Ident,
        visibility: VisibilityKind,
        span: Span,
        kind: TypeDefKind,
    ) {
        if self.reject_non_shadowable(name) {
            return;
        }
        if let Some(previous) = self.modules[module.index()].types.get(&name.name).copied() {
            let earlier = self.context.def(previous).span;
            self.report(duplicate(name, earlier, "type"));
            return;
        }
        let id = self.context.next_def_id();
        self.context.add_def(TypeDef {
            id,
            name: name.name.clone(),
            module,
            visibility,
            generics: Vec::new(),
            obj_kind: kind,
            span,
        });
        self.modules[module.index()]
            .types
            .insert(name.name.clone(), id);
    }

    fn declare_function(&mut self, module: ModuleId, position: usize, decl: &FunctionDecl) {
        if self.reject_non_shadowable(&decl.name) {
            return;
        }
        if let Some(previous) = self.modules[module.index()]
            .functions
            .get(&decl.name.name)
            .copied()
        {
            let earlier = self.context.func(previous).span;
            self.report(duplicate(&decl.name, earlier, "function"));
            return;
        }
        let id = self.context.next_func_id();
        self.context.add_func(FuncDef {
            id,
            name: decl.name.name.clone(),
            module,
            owner: None,
            visibility: decl.visibility.kind,
            generics: Vec::new(),
            params: Vec::new(),
            ret: TypeId::VOID,
            failable: false,
            has_receiver: false,
            has_body: true,
            span: decl.span,
        });
        self.modules[module.index()]
            .functions
            .insert(decl.name.name.clone(), id);
        self.functions.insert(
            id,
            FuncLocation {
                module,
                declaration: position,
                member: None,
            },
        );
    }

    fn declare_module_constant(&mut self, module: ModuleId, position: usize, decl: &ConstDecl) {
        let mut path = Vec::new();
        let bindings = collect_pattern_bindings(&decl.pattern, &mut path);
        for (name, path) in bindings {
            if self.reject_non_shadowable(&name) {
                continue;
            }
            if let Some(previous) = self.modules[module.index()]
                .constants
                .get(&name.name)
                .copied()
            {
                let earlier = self.globals[previous.index()].span;
                self.report(duplicate(&name, earlier, "constant"));
                continue;
            }
            let id = GlobalConstId(self.globals.len() as u32);
            self.globals.push(GlobalConst {
                id,
                name: name.name.clone(),
                module,
                visibility: decl.visibility.kind,
                declaration: position,
                path,
                span: name.span,
            });
            self.modules[module.index()]
                .constants
                .insert(name.name.clone(), id);
        }
    }

    fn reject_non_shadowable(&mut self, name: &Ident) -> bool {
        if NON_SHADOWABLE.contains(&name.name.as_str()) {
            self.report(
                CompileError::at(
                    ErrorCode::ShadowsPredeclaredName,
                    name.span,
                    format!("`{}` is predeclared and cannot be redeclared", name.name),
                )
                .with_note(format!(
                    "the non-shadowable predeclared names are {}",
                    NON_SHADOWABLE.join(", ")
                )),
            );
            return true;
        }
        false
    }
}

fn module_key(segments: &[String]) -> String {
    segments.join("\\")
}

fn duplicate(name: &Ident, earlier: Span, what: &str) -> CompileError {
    CompileError::at(
        ErrorCode::DuplicateDeclaration,
        name.span,
        format!(
            "`{}` is already declared as a {what} in this module",
            name.name
        ),
    )
    .with_secondary(earlier, "first declared here")
}

fn collect_pattern_bindings(
    pattern: &IrrefutablePattern,
    path: &mut Vec<u32>,
) -> Vec<(Ident, Vec<u32>)> {
    match &pattern.kind {
        IrrefutablePatternKind::Binding(name) => vec![(name.clone(), path.clone())],
        IrrefutablePatternKind::Wildcard => Vec::new(),
        IrrefutablePatternKind::Tuple(elements) => {
            let mut out = Vec::new();
            for (index, element) in elements.iter().enumerate() {
                path.push(index as u32);
                out.extend(collect_pattern_bindings(element, path));
                path.pop();
            }
            out
        }
    }
}

// luot 3: tham so generic, than cua kieu, va chu ky

impl Resolver<'_> {
    fn resolve_generic_parameters(&mut self) {
        for index in 1..self.units.len() {
            let module = ModuleId(index as u32);
            self.module = module;
            let declarations = self.units[module.index()].declarations.clone();
            for declaration in &declarations {
                let (name, generics) = match declaration {
                    Declaration::Struct(decl) => (&decl.name, &decl.generics),
                    Declaration::Enum(decl) => (&decl.name, &decl.generics),
                    Declaration::Interface(decl) => (&decl.name, &decl.generics),
                    _ => continue,
                };
                let Some(def) = self.modules[module.index()].types.get(&name.name).copied() else {
                    continue;
                };
                let resolved = self.build_generic_params(generics, GenericOwner::Type(def));
                self.context.def_mut(def).generics = resolved;
            }
        }
    }

    fn build_generic_params(
        &mut self,
        params: &[GenericParam],
        owner: GenericOwner,
    ) -> Vec<GenericParamDef> {
        let mut seen: HashMap<String, Span> = HashMap::new();
        let mut out = Vec::new();
        for (index, param) in params.iter().enumerate() {
            if self.reject_non_shadowable(&param.name) {
                continue;
            }
            if let Some(&earlier) = seen.get(&param.name.name) {
                self.report(
                    CompileError::at(
                        ErrorCode::DuplicateGenericParameter,
                        param.name.span,
                        format!("`{}` is already a generic parameter here", param.name.name),
                    )
                    .with_secondary(earlier, "first declared here"),
                );
                continue;
            }
            seen.insert(param.name.name.clone(), param.name.span);

            let mut bounds = Vec::new();
            for bound in &param.bounds {
                match self.lookup_type_path(bound) {
                    Some(def) if self.context.def(def).as_interface().is_some() => {
                        bounds.push(def);
                    }
                    Some(def) => {
                        let name = self.context.def(def).name.clone();
                        self.report(
                            CompileError::at(
                                ErrorCode::UnknownInterface,
                                bound.span,
                                format!("`{name}` is not an interface"),
                            )
                            .with_help("a generic bound must name an interface"),
                        );
                    }
                    None => self.report(CompileError::at(
                        ErrorCode::UnknownInterface,
                        bound.span,
                        format!("cannot find interface `{}` in scope", bound.name.name),
                    )),
                }
            }

            out.push(GenericParamDef {
                name: param.name.name.clone(),
                bounds,
                span: param.span,
            });
            let _ = (index, owner);
        }
        out
    }

    fn push_generics(&mut self, owner: GenericOwner) {
        let names: Vec<String> = match owner {
            GenericOwner::Type(def) => self
                .context
                .def(def)
                .generics
                .iter()
                .map(|g| g.name.clone())
                .collect(),
            GenericOwner::Func(func) => self
                .context
                .func(func)
                .generics
                .iter()
                .map(|g| g.name.clone())
                .collect(),
        };
        let scope = names
            .into_iter()
            .enumerate()
            .map(|(index, name)| {
                (
                    name,
                    GenericId {
                        owner,
                        index: index as u32,
                    },
                )
            })
            .collect();
        self.generic_scopes.push(scope);
    }

    fn pop_generics(&mut self) {
        self.generic_scopes.pop();
    }

    fn resolve_type_bodies(&mut self) {
        for index in 1..self.units.len() {
            let module = ModuleId(index as u32);
            self.module = module;
            let declarations = self.units[module.index()].declarations.clone();
            for declaration in &declarations {
                match declaration {
                    Declaration::Struct(decl) => self.resolve_struct_body(module, decl),
                    Declaration::Enum(decl) => self.resolve_enum_body(module, decl),
                    _ => {}
                }
            }
        }
    }

    fn resolve_struct_body(&mut self, module: ModuleId, decl: &StructDecl) {
        let Some(def) = self.modules[module.index()]
            .types
            .get(&decl.name.name)
            .copied()
        else {
            return;
        };
        self.push_generics(GenericOwner::Type(def));

        let mut fields: Vec<FieldDef> = Vec::new();
        let mut names: HashMap<String, Span> = HashMap::new();
        for member in &decl.members {
            let name = member.name().clone();
            if let Some(&earlier) = names.get(&name.name) {
                let code = match member {
                    StructMember::Field(_) => ErrorCode::DuplicateField,
                    StructMember::Method(_) => ErrorCode::DuplicateMethod,
                };
                self.report(
                    CompileError::at(
                        code,
                        name.span,
                        format!(
                            "`{}` is already a member of `{}`",
                            name.name, decl.name.name
                        ),
                    )
                    .with_secondary(earlier, "first declared here")
                    .with_note("fields and methods share one namespace (grammar 12.2.4)"),
                );
                continue;
            }
            names.insert(name.name.clone(), name.span);

            if let StructMember::Field(field) = member {
                let ty = self.resolve_type(&field.ty, TypePosition::Value);
                fields.push(FieldDef {
                    name: field.name.name.clone(),
                    ty,
                    visibility: field.visibility.kind,
                    span: field.span,
                });
            }
        }

        self.pop_generics();
        if let TypeDefKind::Struct(body) = &mut self.context.def_mut(def).obj_kind {
            body.fields = fields;
        }
    }

    fn resolve_enum_body(&mut self, module: ModuleId, decl: &EnumDecl) {
        let Some(def) = self.modules[module.index()]
            .types
            .get(&decl.name.name)
            .copied()
        else {
            return;
        };
        self.push_generics(GenericOwner::Type(def));

        let enum_visibility = decl.visibility.kind;
        let mut variants: Vec<VariantDef> = Vec::new();
        let mut names: HashMap<String, Span> = HashMap::new();
        for member in &decl.members {
            let name = member.name().clone();
            if let Some(&earlier) = names.get(&name.name) {
                let code = match member {
                    EnumMember::Variant(_) => ErrorCode::DuplicateVariant,
                    EnumMember::Method(_) => ErrorCode::DuplicateMethod,
                };
                self.report(
                    CompileError::at(
                        code,
                        name.span,
                        format!(
                            "`{}` is already a member of `{}`",
                            name.name, decl.name.name
                        ),
                    )
                    .with_secondary(earlier, "first declared here"),
                );
                continue;
            }
            names.insert(name.name.clone(), name.span);

            if let EnumMember::Variant(variant) = member {
                if variant.visibility.span.is_some() && variant.visibility.kind != enum_visibility {
                    self.report(
                        CompileError::at(
                            ErrorCode::VariantVisibilityMismatch,
                            variant.span,
                            format!(
                                "`{}` is declared {} but `{}` is {}",
                                variant.name.name,
                                describe_visibility(variant.visibility.kind),
                                decl.name.name,
                                describe_visibility(enum_visibility),
                            ),
                        )
                        .with_help(
                            "a variant is exactly as visible as its enum; omit the modifier",
                        ),
                    );
                }
                let payload = variant
                    .payload
                    .iter()
                    .map(|ty| self.resolve_type(ty, TypePosition::Value))
                    .collect();
                variants.push(VariantDef {
                    name: variant.name.name.clone(),
                    payload,
                    span: variant.span,
                });
            }
        }

        if variants.is_empty() {
            self.report(CompileError::at(
                ErrorCode::EnumWithoutVariants,
                decl.span,
                format!("`{}` declares no variants", decl.name.name),
            ));
        }

        self.pop_generics();
        if let TypeDefKind::Enum(body) = &mut self.context.def_mut(def).obj_kind {
            body.variants = variants;
        }
    }

    fn resolve_signatures(&mut self) {
        for index in 1..self.units.len() {
            let module = ModuleId(index as u32);
            self.module = module;
            let declarations = self.units[module.index()].declarations.clone();
            for (position, declaration) in declarations.iter().enumerate() {
                match declaration {
                    Declaration::Function(decl) => {
                        let Some(id) = self.modules[module.index()]
                            .functions
                            .get(&decl.name.name)
                            .copied()
                        else {
                            continue;
                        };
                        self.resolve_function_signature(id, decl);
                    }
                    Declaration::Struct(decl) => {
                        self.resolve_member_signatures(
                            module,
                            position,
                            &decl.name,
                            decl.members
                                .iter()
                                .enumerate()
                                .filter_map(|(i, m)| match m {
                                    StructMember::Method(method) => Some((i, method.clone())),
                                    StructMember::Field(_) => None,
                                }),
                        );
                    }
                    Declaration::Enum(decl) => {
                        self.resolve_member_signatures(
                            module,
                            position,
                            &decl.name,
                            decl.members
                                .iter()
                                .enumerate()
                                .filter_map(|(i, m)| match m {
                                    EnumMember::Method(method) => Some((i, method.clone())),
                                    EnumMember::Variant(_) => None,
                                }),
                        );
                    }
                    Declaration::Interface(decl) => {
                        self.resolve_interface_signatures(module, decl);
                    }
                    _ => {}
                }
            }
        }
    }

    fn resolve_member_signatures(
        &mut self,
        module: ModuleId,
        position: usize,
        owner_name: &Ident,
        methods: impl Iterator<Item = (usize, FunctionDecl)>,
    ) {
        let Some(def) = self.modules[module.index()]
            .types
            .get(&owner_name.name)
            .copied()
        else {
            return;
        };
        self.push_generics(GenericOwner::Type(def));
        let mut ids = Vec::new();
        for (member_index, method) in methods {
            let id = self.context.next_func_id();
            self.context.add_func(FuncDef {
                id,
                name: method.name.name.clone(),
                module,
                owner: Some(def),
                visibility: method.visibility.kind,
                generics: Vec::new(),
                params: Vec::new(),
                ret: TypeId::VOID,
                failable: false,
                has_receiver: true,
                has_body: true,
                span: method.span,
            });
            self.functions.insert(
                id,
                FuncLocation {
                    module,
                    declaration: position,
                    member: Some(member_index),
                },
            );
            self.resolve_function_signature(id, &method);
            ids.push(id);
        }
        self.pop_generics();

        let obj = self.context.def_mut(def);
        match &mut obj.obj_kind {
            TypeDefKind::Struct(body) => body.methods = ids,
            TypeDefKind::Enum(body) => body.methods = ids,
            TypeDefKind::Interface(body) => body.methods = ids,
        }
    }

    fn resolve_interface_signatures(&mut self, module: ModuleId, decl: &InterfaceDecl) {
        let Some(def) = self.modules[module.index()]
            .types
            .get(&decl.name.name)
            .copied()
        else {
            return;
        };
        self.push_generics(GenericOwner::Type(def));

        let mut ids = Vec::new();
        let mut names: HashMap<String, Span> = HashMap::new();
        for method in &decl.methods {
            if let Some(&earlier) = names.get(&method.name.name) {
                self.report(
                    CompileError::at(
                        ErrorCode::DuplicateMethod,
                        method.name.span,
                        format!(
                            "`{}` is already declared in `{}`",
                            method.name.name, decl.name.name
                        ),
                    )
                    .with_secondary(earlier, "first declared here"),
                );
                continue;
            }
            names.insert(method.name.name.clone(), method.name.span);

            let id = self.context.next_func_id();
            self.context.add_func(FuncDef {
                id,
                name: method.name.name.clone(),
                module,
                owner: Some(def),
                visibility: decl.visibility.kind,
                generics: Vec::new(),
                params: Vec::new(),
                ret: TypeId::VOID,
                failable: false,
                has_receiver: true,
                has_body: false,
                span: method.span,
            });

            let generics = self.build_generic_params(&method.generics, GenericOwner::Func(id));
            self.context.func_mut(id).generics = generics;
            self.push_generics(GenericOwner::Func(id));

            let params = self.resolve_params(&method.params, true);
            let (ret, failable) = self.resolve_return_type(method.return_type.as_ref());
            self.pop_generics();

            let func = self.context.func_mut(id);
            func.params = params;
            func.ret = ret;
            func.failable = failable;
            ids.push(id);
        }

        self.pop_generics();
        if let TypeDefKind::Interface(body) = &mut self.context.def_mut(def).obj_kind {
            body.methods = ids;
        }
    }

    fn resolve_function_signature(&mut self, id: FuncId, decl: &FunctionDecl) {
        let generics = self.build_generic_params(&decl.generics, GenericOwner::Func(id));
        self.context.func_mut(id).generics = generics;
        self.push_generics(GenericOwner::Func(id));

        let params = self.resolve_params(&decl.params, false);
        let (ret, failable) = self.resolve_return_type(decl.return_type.as_ref());

        self.pop_generics();
        let func = self.context.func_mut(id);
        func.params = params;
        func.ret = ret;
        func.failable = failable;
    }

    fn resolve_params(&mut self, params: &[Param], interface_method: bool) -> Vec<ParamDef> {
        let mut out = Vec::new();
        let mut names: HashMap<String, Span> = HashMap::new();
        for param in params {
            if let Some(&earlier) = names.get(&param.name.name) {
                self.report(
                    CompileError::at(
                        ErrorCode::DuplicateParameter,
                        param.name.span,
                        format!(
                            "`{}` is already a parameter of this function",
                            param.name.name
                        ),
                    )
                    .with_secondary(earlier, "first declared here"),
                );
                continue;
            }
            names.insert(param.name.name.clone(), param.name.span);

            if interface_method {
                if let ParamKind::Default(value) = &param.kind {
                    self.report(
                        CompileError::at(
                            ErrorCode::InterfaceMethodDefaultParameter,
                            value.span,
                            "an interface method signature may not give a default value",
                        )
                        .with_note(
                            "defaults are not part of structural conformance (grammar 12.4.3)",
                        ),
                    );
                }
            }

            let ty = self.resolve_type(&param.ty, TypePosition::Value);
            out.push(ParamDef {
                name: param.name.name.clone(),
                ty,
                default: None,
                variadic: matches!(param.kind, ParamKind::Variadic),
                span: param.span,
            });
        }
        out
    }

    fn resolve_return_type(&mut self, written: Option<&TypeExpr>) -> (TypeId, bool) {
        let Some(written) = written else {
            return (TypeId::VOID, false);
        };
        let resolved = self.resolve_type(written, TypePosition::Return);
        match self.context.kind(resolved).clone() {
            TypeKind::Failable(inner) => (inner, true),
            _ => (resolved, false),
        }
    }
}

fn describe_visibility(kind: VisibilityKind) -> &'static str {
    match kind {
        VisibilityKind::Public => "pub",
        VisibilityKind::Private => "private",
    }
}

// kieu, dung nhu source viet (22)

impl Resolver<'_> {
    fn resolve_type(&mut self, written: &TypeExpr, position: TypePosition) -> TypeId {
        let resolved = self.resolve_type_uncached(written, position);
        self.types.insert(written.id, resolved);
        resolved
    }

    fn resolve_type_uncached(&mut self, written: &TypeExpr, position: TypePosition) -> TypeId {
        match &written.kind {
            TypeExprKind::Path { path, args } => self.resolve_type_path(written, path, args),
            TypeExprKind::Array(element) => {
                let element = self.resolve_type(element, TypePosition::Value);
                self.context.array_of(element)
            }
            TypeExprKind::Map { key, value } => {
                let key = self.resolve_type(key, TypePosition::Value);
                let value = self.resolve_type(value, TypePosition::Value);
                if !self.context.is_hashable(key) && key != TypeId::ERROR {
                    let shown = self.context.display(key);
                    let code = if key == TypeId::FLOAT {
                        ErrorCode::FloatNotHashable
                    } else {
                        ErrorCode::TypeMismatch
                    };
                    self.report(
                        CompileError::at(
                            code,
                            written.span,
                            format!("`{shown}` cannot be a map key"),
                        )
                        .with_note(
                            "keys must be `bool`, an integer, `char`, `string`, a tuple of those, \
                             or a payload-free enum",
                        ),
                    );
                }
                self.context.map_of(key, value)
            }
            TypeExprKind::Set(element) => {
                let element = self.resolve_type(element, TypePosition::Value);
                if !self.context.is_hashable(element) && element != TypeId::ERROR {
                    let shown = self.context.display(element);
                    let code = if element == TypeId::FLOAT {
                        ErrorCode::FloatNotHashable
                    } else {
                        ErrorCode::TypeMismatch
                    };
                    self.report(CompileError::at(
                        code,
                        written.span,
                        format!("`{shown}` cannot be a set element"),
                    ));
                }
                self.context.set_of(element)
            }
            TypeExprKind::Tuple(elements) => {
                let elements = elements
                    .iter()
                    .map(|element| self.resolve_type(element, TypePosition::Value))
                    .collect();
                self.context.tuple_of(elements)
            }
            TypeExprKind::Function(signature) => {
                let params = signature
                    .params
                    .iter()
                    .map(|param| self.resolve_type(param, TypePosition::Value))
                    .collect();
                let variadic = signature
                    .variadic
                    .as_ref()
                    .map(|element| self.resolve_type(element, TypePosition::Value));
                let (ret, failable) = self.resolve_return_type(signature.return_type.as_deref());
                self.context.function(FnType {
                    params,
                    variadic,
                    ret,
                    failable,
                })
            }
            TypeExprKind::Optional(inner) => {
                let inner = self.resolve_type(inner, position);
                match self.context.kind(inner) {
                    TypeKind::Optional(_) => {
                        self.report(
                            CompileError::at(
                                ErrorCode::NestedOptional,
                                written.span,
                                "an optional of an optional has no extra state to represent",
                            )
                            .with_help("write the type once with a single `?`"),
                        );
                        inner
                    }
                    TypeKind::Void => {
                        self.report(CompileError::at(
                            ErrorCode::ExpectedType,
                            written.span,
                            "`void?` is not a type; `void` is the absence of a value",
                        ));
                        TypeId::ERROR
                    }
                    _ => self.context.optional_of(inner),
                }
            }
            TypeExprKind::Failable(inner) => {
                let inner_ty = self.resolve_type(inner, position);
                if position != TypePosition::Return {
                    self.report(
                        CompileError::at(
                            ErrorCode::ErrorTypeOutsideReturn,
                            written.span,
                            "`!` may only be used on a function's return type",
                        )
                        .with_help("handle the error at the call site with `!` or `catch`"),
                    );
                    return inner_ty;
                }
                match self.context.kind(inner_ty) {
                    TypeKind::Failable(_) => {
                        self.report(
                            CompileError::at(
                                ErrorCode::NestedErrorType,
                                written.span,
                                "an error of an error has no extra state to represent",
                            )
                            .with_help("write the type once with a single `!`"),
                        );
                        inner_ty
                    }
                    _ => self.context.failable_of(inner_ty),
                }
            }
            TypeExprKind::Group(inner) => self.resolve_type(inner, position),
        }
    }

    fn resolve_type_path(
        &mut self,
        written: &TypeExpr,
        path: &TypePath,
        args: &[TypeExpr],
    ) -> TypeId {
        // duong dan mot doan co the la ten tham so generic hoac ten kieu co
        // ban truoc khi no la ten mot khai bao (16.8)
        if path.module.is_none() {
            if let Some(generic) = self.lookup_generic(&path.name.name) {
                if !args.is_empty() {
                    self.report(CompileError::at(
                        ErrorCode::WrongTypeArgumentCount,
                        written.span,
                        format!(
                            "`{}` is a type parameter and takes no arguments",
                            path.name.name
                        ),
                    ));
                }
                return self.context.intern(TypeKind::Generic(generic));
            }
            if let Some(primitive) = primitive_type(&path.name.name) {
                if !args.is_empty() {
                    self.report(CompileError::at(
                        ErrorCode::WrongTypeArgumentCount,
                        written.span,
                        format!("`{}` takes no type arguments", path.name.name),
                    ));
                }
                if primitive == TypeId::VOID {
                    // `void` chi hop le o cho doi mot kieu tra ve, ma
                    // `resolve_return_type` la nguoi goi duy nhat hoi cai do,
                    // nen toi day o vi tri Value la loi
                    return TypeId::VOID;
                }
                return primitive;
            }
        }

        let Some(def) = self.lookup_type_path(path) else {
            let mut error = CompileError::at(
                ErrorCode::UnknownType,
                path.span,
                format!("cannot find type `{}` in scope", render_type_path(path)),
            );
            if let Some(suggestion) = self.suggest_type_name(&path.name.name) {
                error = error.with_help(format!("a type named `{suggestion}` is in scope"));
            }
            self.report(error);
            for arg in args {
                self.resolve_type(arg, TypePosition::Value);
            }
            return TypeId::ERROR;
        };

        self.check_type_visibility(def, path.span);

        let expected = self.context.def(def).generics.len();
        let mut resolved: Vec<TypeId> = args
            .iter()
            .map(|arg| self.resolve_type(arg, TypePosition::Value))
            .collect();
        if resolved.len() != expected {
            let name = self.context.def(def).name.clone();
            self.report(
                CompileError::at(
                    ErrorCode::WrongTypeArgumentCount,
                    written.span,
                    format!(
                        "`{name}` takes {expected} type argument{}, but {} {} supplied",
                        if expected == 1 { "" } else { "s" },
                        resolved.len(),
                        if resolved.len() == 1 { "was" } else { "were" },
                    ),
                )
                .with_secondary(self.context.def(def).span, "declared here"),
            );
            resolved.resize(expected, TypeId::ERROR);
        }
        self.context.named(def, resolved)
    }

    fn lookup_type_path(&mut self, path: &TypePath) -> Option<DefId> {
        match &path.module {
            Some(alias) => {
                let target = self.use_import(alias)?;
                self.modules[target.index()]
                    .types
                    .get(&path.name.name)
                    .copied()
            }
            None => {
                if let Some(def) = self.modules[self.module.index()]
                    .types
                    .get(&path.name.name)
                    .copied()
                {
                    return Some(def);
                }
                self.prelude_type(&path.name.name)
            }
        }
    }

    fn prelude_type(&self, name: &str) -> Option<DefId> {
        let prelude = self.prelude();
        match name {
            "Error" => Some(prelude.error),
            "Stringable" => Some(prelude.stringable),
            "Range" => Some(prelude.range),
            _ => None,
        }
    }

    fn lookup_generic(&self, name: &str) -> Option<GenericId> {
        self.generic_scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
    }

    fn use_import(&mut self, alias: &Ident) -> Option<ModuleId> {
        let module = self.module.index();
        match self.modules[module].imports.get_mut(&alias.name) {
            Some(binding) => {
                binding.used = true;
                Some(binding.module)
            }
            None => {
                self.report(CompileError::at(
                    ErrorCode::UnknownModule,
                    alias.span,
                    format!("no module named `{}` is imported here", alias.name),
                ));
                None
            }
        }
    }

    fn check_type_visibility(&mut self, def: DefId, span: Span) {
        let definition = self.context.def(def);
        if definition.module == self.module || definition.visibility == VisibilityKind::Public {
            return;
        }
        let name = definition.name.clone();
        let declared = definition.span;
        let owner = self.modules[definition.module.index()].path.join("\\");
        self.report(
            CompileError::at(
                ErrorCode::PrivateAccess,
                span,
                format!("`{name}` is private to module `{owner}`"),
            )
            .with_secondary(declared, "declared here, without `pub`")
            .with_help(format!("mark it `pub struct {name}` to export it")),
        );
    }

    fn suggest_type_name(&self, name: &str) -> Option<String> {
        let candidates = self.modules[self.module.index()].types.keys();
        closest(name, candidates.map(|key| key.as_str()))
    }
}

fn render_type_path(path: &TypePath) -> String {
    match &path.module {
        Some(module) => format!("{}.{}", module.name, path.name.name),
        None => path.name.name.clone(),
    }
}

fn closest<'a>(name: &str, candidates: impl Iterator<Item = &'a str>) -> Option<String> {
    let limit = match name.len() {
        0..=3 => 1,
        4..=7 => 2,
        _ => 3,
    };
    let mut best: Option<(usize, &str)> = None;
    for candidate in candidates {
        let distance = edit_distance(name, candidate);
        if distance <= limit && best.is_none_or(|(score, _)| distance < score) {
            best = Some((distance, candidate));
        }
    }
    best.map(|(_, candidate)| candidate.to_string())
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0usize; b.len() + 1];
    for (i, &left) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, &right) in b.iter().enumerate() {
            let substitution = previous[j] + usize::from(left != right);
            current[j + 1] = substitution.min(previous[j + 1] + 1).min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

// `implements` va gia tri mac dinh cua tham so

impl Resolver<'_> {
    fn resolve_implements(&mut self) {
        for index in 1..self.units.len() {
            let module = ModuleId(index as u32);
            self.module = module;
            let declarations = self.units[module.index()].declarations.clone();
            for declaration in &declarations {
                let Declaration::Implements(decl) = declaration else {
                    continue;
                };
                let Some(subject) = self.modules[module.index()]
                    .types
                    .get(&decl.subject.name)
                    .copied()
                else {
                    self.report(CompileError::at(
                        ErrorCode::UnknownType,
                        decl.subject.span,
                        format!("cannot find type `{}` in this module", decl.subject.name),
                    ));
                    continue;
                };
                if self.context.def(subject).is_generic() {
                    self.report(
                        CompileError::at(
                            ErrorCode::ImplementsGenericSubject,
                            decl.subject.span,
                            format!("`{}` is generic", decl.subject.name),
                        )
                        .with_note("`implements` on a generic type is deferred past Pump 1.0"),
                    );
                    continue;
                }

                let mut interfaces = Vec::new();
                for path in &decl.interfaces {
                    match self.lookup_type_path(path) {
                        Some(def) if self.context.def(def).as_interface().is_some() => {
                            interfaces.push((def, path.span));
                        }
                        Some(def) => {
                            let name = self.context.def(def).name.clone();
                            self.report(CompileError::at(
                                ErrorCode::UnknownInterface,
                                path.span,
                                format!("`{name}` is not an interface"),
                            ));
                        }
                        None => self.report(CompileError::at(
                            ErrorCode::UnknownInterface,
                            path.span,
                            format!(
                                "cannot find interface `{}` in scope",
                                render_type_path(path)
                            ),
                        )),
                    }
                }

                self.implements.push(ImplementsAssertion {
                    subject,
                    subject_span: decl.subject.span,
                    interfaces,
                    span: decl.span,
                });
            }
        }
    }

    fn fold_parameter_defaults(&mut self) {
        let mut work: Vec<(FuncId, usize, Expr, TypeId, ModuleId)> = Vec::new();
        for (&func, location) in &self.functions {
            let Some(decl) = declaration_of(&self.units, *location) else {
                continue;
            };
            let definition = self.context.func(func);
            for (index, param) in decl.params.iter().enumerate() {
                if let ParamKind::Default(value) = &param.kind {
                    let Some(declared) = definition.params.get(index) else {
                        continue;
                    };
                    work.push((func, index, value.clone(), declared.ty, location.module));
                }
            }
        }
        work.sort_by_key(|(func, index, ..)| (*func, *index));

        for (func, index, value, ty, module) in work {
            self.module = module;
            let folded = self.fold_constant(&value, &mut Vec::new());
            let Some(folded) = folded else { continue };
            let Some(coerced) = self.coerce_constant(folded, ty, value.span) else {
                continue;
            };
            self.context.func_mut(func).params[index].default = Some(coerced);
        }
    }

    fn coerce_constant(
        &mut self,
        value: ConstValue,
        target: TypeId,
        span: Span,
    ) -> Option<ConstValue> {
        let kind = self.context.kind(target).clone();
        let coerced = match (&value, &kind) {
            (ConstValue::Int(n), TypeKind::Uint) => match u64::try_from(*n) {
                Ok(n) => ConstValue::Uint(n),
                Err(_) => {
                    self.report(CompileError::at(
                        ErrorCode::LiteralOutOfRange,
                        span,
                        format!("`{n}` is negative and cannot be a `uint`"),
                    ));
                    return None;
                }
            },
            (ConstValue::Int(n), TypeKind::Float) => ConstValue::Float(*n as f64),
            _ => value,
        };
        Some(coerced)
    }
}

fn declaration_of(units: &[SourceUnit], location: FuncLocation) -> Option<&FunctionDecl> {
    let unit = units.get(location.module.index())?;
    match (
        unit.declarations.get(location.declaration)?,
        location.member,
    ) {
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

// gap hang so (12.6)

impl Resolver<'_> {
    fn fold_constant(
        &mut self,
        expr: &Expr,
        visiting: &mut Vec<GlobalConstId>,
    ) -> Option<ConstValue> {
        match &expr.kind {
            ExprKind::Int(value) => Some(ConstValue::Int(*value as i64)),
            ExprKind::Float(value) => Some(ConstValue::Float(*value)),
            ExprKind::Char(value) => Some(ConstValue::Char(*value)),
            ExprKind::Bool(value) => Some(ConstValue::Bool(*value)),
            ExprKind::Null => Some(ConstValue::Null),
            ExprKind::Str(literal) => match literal.as_plain() {
                Some(text) => Some(ConstValue::Str(text)),
                None => {
                    self.report(
                        CompileError::at(
                            ErrorCode::NotAConstantExpression,
                            expr.span,
                            "string interpolation is evaluated at run time",
                        )
                        .with_help("build the string in the function body instead"),
                    );
                    None
                }
            },
            ExprKind::Group(inner) => self.fold_constant(inner, visiting),
            ExprKind::Ident(name) => self.fold_constant_reference(name, expr.span, visiting),
            ExprKind::Array(elements) => {
                let mut out = Vec::with_capacity(elements.len());
                for element in elements {
                    out.push(self.fold_constant(element, visiting)?);
                }
                Some(ConstValue::Array(out))
            }
            ExprKind::Tuple(elements) => {
                let mut out = Vec::with_capacity(elements.len());
                for element in elements {
                    out.push(self.fold_constant(element, visiting)?);
                }
                Some(ConstValue::Tuple(out))
            }
            ExprKind::Set(elements) => {
                let mut out = Vec::with_capacity(elements.len());
                for element in elements {
                    out.push(self.fold_constant(element, visiting)?);
                }
                Some(ConstValue::Set(out))
            }
            ExprKind::Map(entries) => {
                let mut out = Vec::with_capacity(entries.len());
                for entry in entries {
                    let key = self.fold_constant(&entry.key, visiting)?;
                    let value = self.fold_constant(&entry.value, visiting)?;
                    out.push((key, value));
                }
                Some(ConstValue::Map(out))
            }
            ExprKind::Field { base, name } => self.fold_constant_variant(base, name, expr.span),
            ExprKind::Unary { op, operand } => {
                let value = self.fold_constant(operand, visiting)?;
                self.fold_unary(*op, value, expr.span)
            }
            ExprKind::Binary { op, lhs, rhs } => {
                let left = self.fold_constant(lhs, visiting)?;
                let right = self.fold_constant(rhs, visiting)?;
                self.fold_binary(*op, left, right, expr.span)
            }
            _ => {
                self.report(
                    CompileError::at(
                        ErrorCode::NotAConstantExpression,
                        expr.span,
                        "this cannot be evaluated at compile time",
                    )
                    .with_note(
                        "a default may use literals, other module constants, operators, and \
                         literal collections (grammar 12.6.1)",
                    ),
                );
                None
            }
        }
    }

    fn fold_constant_reference(
        &mut self,
        name: &Ident,
        span: Span,
        visiting: &mut Vec<GlobalConstId>,
    ) -> Option<ConstValue> {
        let Some(global) = self.modules[self.module.index()]
            .constants
            .get(&name.name)
            .copied()
        else {
            self.report(
                CompileError::at(
                    ErrorCode::UnknownIdentifier,
                    span,
                    format!("cannot find constant `{}` in scope", name.name),
                )
                .with_note("a parameter default may reference only module-level constants"),
            );
            return None;
        };
        if visiting.contains(&global) {
            self.report(CompileError::at(
                ErrorCode::CircularConstInitialisation,
                span,
                format!("`{}` is defined in terms of itself", name.name),
            ));
            return None;
        }

        let entry = &self.globals[global.index()];
        if !entry.path.is_empty() {
            self.report(
                CompileError::at(
                    ErrorCode::NotAConstantExpression,
                    span,
                    format!("`{}` comes from a destructuring constant", name.name),
                )
                .with_help("give the default a literal value instead"),
            );
            return None;
        }
        let module = entry.module;
        let declaration = entry.declaration;
        let Some(Declaration::Const(decl)) = self.units[module.index()]
            .declarations
            .get(declaration)
            .cloned()
        else {
            return None;
        };

        let outer = self.module;
        self.module = module;
        visiting.push(global);
        let folded = self.fold_constant(&decl.value, visiting);
        visiting.pop();
        self.module = outer;
        folded
    }

    fn fold_constant_variant(
        &mut self,
        base: &Expr,
        variant: &Ident,
        span: Span,
    ) -> Option<ConstValue> {
        let ExprKind::Ident(type_name) = &base.kind else {
            self.report(CompileError::at(
                ErrorCode::NotAConstantExpression,
                span,
                "field access is evaluated at run time",
            ));
            return None;
        };
        let path = TypePath {
            module: None,
            name: type_name.clone(),
            span: type_name.span,
        };
        let Some(def) = self.lookup_type_path(&path) else {
            self.report(CompileError::at(
                ErrorCode::NotAConstantExpression,
                span,
                "field access is evaluated at run time",
            ));
            return None;
        };
        let Some(enumeration) = self.context.def(def).as_enum() else {
            self.report(CompileError::at(
                ErrorCode::NotAnEnum,
                span,
                format!("`{}` is not an enum", type_name.name),
            ));
            return None;
        };
        let Some(index) = enumeration.variant_index(&variant.name) else {
            self.report(CompileError::at(
                ErrorCode::UnknownVariant,
                variant.span,
                format!("`{}` has no variant `{}`", type_name.name, variant.name),
            ));
            return None;
        };
        if !enumeration.variants[index].payload.is_empty() {
            self.report(
                CompileError::at(
                    ErrorCode::NotAConstantExpression,
                    span,
                    format!("`{}` carries a payload", variant.name),
                )
                .with_help("only a payload-free variant is a constant"),
            );
            return None;
        }
        Some(ConstValue::EnumVariant {
            def,
            variant: index as u32,
        })
    }

    fn fold_unary(
        &mut self,
        op: crate::ast::UnaryOp,
        value: ConstValue,
        span: Span,
    ) -> Option<ConstValue> {
        use crate::ast::UnaryOp;
        match (op, value) {
            (UnaryOp::Neg, ConstValue::Int(n)) => match n.checked_neg() {
                Some(negated) => Some(ConstValue::Int(negated)),
                None => {
                    self.report(CompileError::at(
                        ErrorCode::ConstantOverflow,
                        span,
                        "negating this value overflows `int`",
                    ));
                    None
                }
            },
            (UnaryOp::Neg, ConstValue::Float(f)) => Some(ConstValue::Float(-f)),
            (UnaryOp::Not, ConstValue::Bool(b)) => Some(ConstValue::Bool(!b)),
            (op, value) => {
                let operator = if op == UnaryOp::Not { "!" } else { "-" };
                self.report(CompileError::at(
                    ErrorCode::TypeMismatch,
                    span,
                    format!(
                        "`{operator}` does not apply to {}",
                        describe_constant(&value)
                    ),
                ));
                None
            }
        }
    }

    fn fold_binary(
        &mut self,
        op: crate::ast::BinaryOp,
        left: ConstValue,
        right: ConstValue,
        span: Span,
    ) -> Option<ConstValue> {
        use crate::ast::BinaryOp::*;
        let overflow = |resolver: &mut Self| {
            resolver.report(
                CompileError::at(
                    ErrorCode::ConstantOverflow,
                    span,
                    "this constant expression overflows `int`",
                )
                .with_note("runtime arithmetic wraps, but a wrapped constant is always a typo"),
            );
        };

        match (&left, &right) {
            (ConstValue::Int(a), ConstValue::Int(b)) => {
                let (a, b) = (*a, *b);
                let arithmetic = match op {
                    Add => a.checked_add(b),
                    Sub => a.checked_sub(b),
                    Mul => a.checked_mul(b),
                    Shl => b.try_into().ok().and_then(|shift| a.checked_shl(shift)),
                    Shr => b.try_into().ok().and_then(|shift| a.checked_shr(shift)),
                    BitAnd => Some(a & b),
                    BitXor => Some(a ^ b),
                    BitOr => Some(a | b),
                    Div | Rem => {
                        if b == 0 {
                            self.report(CompileError::at(
                                ErrorCode::DivisionByZeroConstant,
                                span,
                                "the divisor of this constant expression is zero",
                            ));
                            return None;
                        }
                        if op == Div {
                            a.checked_div(b)
                        } else {
                            a.checked_rem(b)
                        }
                    }
                    _ => None,
                };
                if let Some(result) = arithmetic {
                    return Some(ConstValue::Int(result));
                }
                if matches!(op, Add | Sub | Mul | Div | Rem | Shl | Shr) {
                    overflow(self);
                    return None;
                }
                match op {
                    Eq => Some(ConstValue::Bool(a == b)),
                    Ne => Some(ConstValue::Bool(a != b)),
                    Lt => Some(ConstValue::Bool(a < b)),
                    Gt => Some(ConstValue::Bool(a > b)),
                    Le => Some(ConstValue::Bool(a <= b)),
                    Ge => Some(ConstValue::Bool(a >= b)),
                    _ => {
                        self.reject_constant_operator(op, &left, span);
                        None
                    }
                }
            }
            (ConstValue::Float(a), ConstValue::Float(b)) => {
                let (a, b) = (*a, *b);
                match op {
                    Add => Some(ConstValue::Float(a + b)),
                    Sub => Some(ConstValue::Float(a - b)),
                    Mul => Some(ConstValue::Float(a * b)),
                    Div => Some(ConstValue::Float(a / b)),
                    Rem => Some(ConstValue::Float(a % b)),
                    Eq => Some(ConstValue::Bool(a == b)),
                    Ne => Some(ConstValue::Bool(a != b)),
                    Lt => Some(ConstValue::Bool(a < b)),
                    Gt => Some(ConstValue::Bool(a > b)),
                    Le => Some(ConstValue::Bool(a <= b)),
                    Ge => Some(ConstValue::Bool(a >= b)),
                    _ => {
                        self.reject_constant_operator(op, &left, span);
                        None
                    }
                }
            }
            (ConstValue::Bool(a), ConstValue::Bool(b)) => match op {
                And => Some(ConstValue::Bool(*a && *b)),
                Or => Some(ConstValue::Bool(*a || *b)),
                Eq => Some(ConstValue::Bool(a == b)),
                Ne => Some(ConstValue::Bool(a != b)),
                _ => {
                    self.reject_constant_operator(op, &left, span);
                    None
                }
            },
            (ConstValue::Str(a), ConstValue::Str(b)) => match op {
                Add => Some(ConstValue::Str(format!("{a}{b}"))),
                Eq => Some(ConstValue::Bool(a == b)),
                Ne => Some(ConstValue::Bool(a != b)),
                Lt => Some(ConstValue::Bool(a < b)),
                Gt => Some(ConstValue::Bool(a > b)),
                Le => Some(ConstValue::Bool(a <= b)),
                Ge => Some(ConstValue::Bool(a >= b)),
                _ => {
                    self.reject_constant_operator(op, &left, span);
                    None
                }
            },
            (ConstValue::Char(a), ConstValue::Char(b)) => match op {
                Eq => Some(ConstValue::Bool(a == b)),
                Ne => Some(ConstValue::Bool(a != b)),
                Lt => Some(ConstValue::Bool(a < b)),
                Gt => Some(ConstValue::Bool(a > b)),
                Le => Some(ConstValue::Bool(a <= b)),
                Ge => Some(ConstValue::Bool(a >= b)),
                _ => {
                    self.reject_constant_operator(op, &left, span);
                    None
                }
            },
            _ => {
                self.report(CompileError::at(
                    ErrorCode::TypeMismatch,
                    span,
                    format!(
                        "cannot apply `{}` to {} and {}",
                        op.spelling(),
                        describe_constant(&left),
                        describe_constant(&right)
                    ),
                ));
                None
            }
        }
    }

    fn reject_constant_operator(
        &mut self,
        op: crate::ast::BinaryOp,
        value: &ConstValue,
        span: Span,
    ) {
        self.report(CompileError::at(
            ErrorCode::TypeMismatch,
            span,
            format!(
                "`{}` does not apply to {}",
                op.spelling(),
                describe_constant(value)
            ),
        ));
    }
}

fn describe_constant(value: &ConstValue) -> &'static str {
    match value {
        ConstValue::Bool(_) => "a `bool`",
        ConstValue::Int(_) => "an integer",
        ConstValue::Uint(_) => "a `uint`",
        ConstValue::Float(_) => "a `float`",
        ConstValue::Char(_) => "a `char`",
        ConstValue::Str(_) => "a `string`",
        ConstValue::Null => "`null`",
        ConstValue::Array(_) => "an array",
        ConstValue::Tuple(_) => "a tuple",
        ConstValue::Map(_) => "a map",
        ConstValue::Set(_) => "a set",
        ConstValue::EnumVariant { .. } => "an enum variant",
    }
}

// luot 4: than ham (16.1)

impl Resolver<'_> {
    fn resolve_bodies(&mut self) {
        for index in 1..self.units.len() {
            let module = ModuleId(index as u32);
            self.module = module;
            let declarations = self.units[module.index()].declarations.clone();
            for declaration in &declarations {
                match declaration {
                    Declaration::Function(decl) => {
                        let id = self.modules[module.index()]
                            .functions
                            .get(&decl.name.name)
                            .copied();
                        self.resolve_function_body(id, None, decl);
                    }
                    Declaration::Struct(decl) => {
                        let owner = self.modules[module.index()]
                            .types
                            .get(&decl.name.name)
                            .copied();
                        self.resolve_methods(
                            owner,
                            decl.members.iter().filter_map(|m| match m {
                                StructMember::Method(method) => Some(method.clone()),
                                StructMember::Field(_) => None,
                            }),
                        );
                    }
                    Declaration::Enum(decl) => {
                        let owner = self.modules[module.index()]
                            .types
                            .get(&decl.name.name)
                            .copied();
                        self.resolve_methods(
                            owner,
                            decl.members.iter().filter_map(|m| match m {
                                EnumMember::Method(method) => Some(method.clone()),
                                EnumMember::Variant(_) => None,
                            }),
                        );
                    }
                    Declaration::Const(decl) => self.resolve_module_constant(module, decl),
                    Declaration::Interface(_) | Declaration::Implements(_) => {}
                }
            }
        }
        self.report_unused_locals();
    }

    fn resolve_methods(
        &mut self,
        owner: Option<DefId>,
        methods: impl Iterator<Item = FunctionDecl>,
    ) {
        let Some(owner) = owner else { return };
        self.push_generics(GenericOwner::Type(owner));
        for method in methods {
            let id = self.context.find_method(owner, &method.name.name);
            self.resolve_function_body(id, Some(owner), &method);
        }
        self.pop_generics();
    }

    fn resolve_function_body(
        &mut self,
        func: Option<FuncId>,
        owner: Option<DefId>,
        decl: &FunctionDecl,
    ) {
        if let Some(func) = func {
            self.push_generics(GenericOwner::Func(func));
        }
        self.frames.push(Frame::new(owner, None));

        for param in &decl.params {
            self.declare_parameter(param);
        }
        self.resolve_block(&decl.body);

        self.frames.pop();
        if func.is_some() {
            self.pop_generics();
        }
    }

    fn resolve_module_constant(&mut self, module: ModuleId, decl: &ConstDecl) {
        let first = collect_pattern_bindings(&decl.pattern, &mut Vec::new())
            .into_iter()
            .next()
            .and_then(|(name, _)| {
                self.modules[module.index()]
                    .constants
                    .get(&name.name)
                    .copied()
            });

        self.frames.push(Frame::new(None, None));
        let previous = self
            .current_const
            .replace(first.unwrap_or(GlobalConstId(u32::MAX)));
        if let Some(ty) = &decl.ty {
            self.resolve_type(ty, TypePosition::Value);
        }
        self.resolve_expr(&decl.value);
        self.current_const = previous;
        self.frames.pop();
    }

    fn declare_parameter(&mut self, param: &Param) {
        if let ParamKind::Default(value) = &param.kind {
            // gap roi. Di vao day nua thi cac identifier cua no se duoc buoc
            // trong sai pham vi, tu gio tro di chi con cai span la con dung.
            let _ = value;
        }
        self.declare_local(&param.name, LocalOrigin::Parameter, true);
    }

    fn declare_local(&mut self, name: &Ident, origin: LocalOrigin, reassignable: bool) -> LocalId {
        let id = LocalId(self.locals.len() as u32);
        self.locals.push(LocalBinding {
            id,
            name: name.name.clone(),
            reassignable,
            captured: false,
            span: name.span,
            origin,
        });
        self.declared_locals.insert(name.span, id);

        if NON_SHADOWABLE.contains(&name.name.as_str()) {
            self.report(
                CompileError::at(
                    ErrorCode::ShadowsPredeclaredName,
                    name.span,
                    format!("`{}` is predeclared and cannot be shadowed", name.name),
                )
                .with_note(format!(
                    "the non-shadowable predeclared names are {}",
                    NON_SHADOWABLE.join(", ")
                )),
            );
            return id;
        }

        let Some(frame) = self.frames.last_mut() else {
            return id;
        };
        let scope = frame
            .blocks
            .last_mut()
            .expect("a frame always has one block scope");
        if let Some(previous) = scope.insert(name.name.clone(), id) {
            let earlier = self.locals[previous.index()].span;
            self.report(
                CompileError::at(
                    ErrorCode::DuplicateDeclaration,
                    name.span,
                    format!("`{}` is already declared in this block", name.name),
                )
                .with_secondary(earlier, "first declared here")
                .with_help("shadowing is allowed in a nested block, not in the same one"),
            );
        }
        id
    }

    fn push_scope(&mut self) {
        if let Some(frame) = self.frames.last_mut() {
            frame.blocks.push(HashMap::new());
        }
    }

    fn pop_scope(&mut self) {
        if let Some(frame) = self.frames.last_mut() {
            frame.blocks.pop();
        }
    }

    // di doc theo scope

    fn lookup_value(&mut self, name: &Ident) -> Option<ValueBinding> {
        // buoc 1 den 3: local cua block va tham so, tu khung trong cung ra
        if let Some((frame_index, local)) = self.find_local(&name.name) {
            self.used_locals.borrow_mut().insert(local);
            let innermost = self.frames.len() - 1;
            if frame_index == innermost {
                return Some(ValueBinding::Local(local));
            }
            self.locals[local.index()].captured = true;
            for frame in &mut self.frames[frame_index + 1..] {
                if frame.closure.is_some() && !frame.captures.contains(&local) {
                    frame.captures.push(local);
                }
            }
            return Some(ValueBinding::Captured(local));
        }

        // buoc 4: truong va method cua `this`
        if let Some(owner) = self.frames.last().and_then(|frame| frame.this_owner) {
            if let Some(binding) = self.lookup_member(owner, &name.name) {
                self.mark_this_captured();
                return Some(binding);
            }
        }

        // buoc 5: khai bao muc module cua chinh file nay
        let module = &self.modules[self.module.index()];
        if let Some(&global) = module.constants.get(&name.name) {
            self.note_constant_dependency(global);
            return Some(ValueBinding::GlobalConst(global));
        }
        if let Some(&func) = module.functions.get(&name.name) {
            return Some(ValueBinding::Function(func));
        }
        if let Some(&def) = module.types.get(&name.name) {
            return Some(ValueBinding::Type(def));
        }

        // buoc 6: alias cua module
        if let Some(binding) = self.modules[self.module.index()]
            .imports
            .get_mut(&name.name)
        {
            binding.used = true;
            return Some(ValueBinding::Module(binding.module));
        }

        // buoc 7: cac gia tri khai bao san, roi den cac phep doi kieu co ban
        if let Some(value) = predeclared_value(&name.name) {
            return Some(ValueBinding::Predeclared(value));
        }
        if let Some(target) = conversion_target(&name.name) {
            return Some(ValueBinding::Conversion(target));
        }
        if let Some(def) = self.prelude_type(&name.name) {
            return Some(ValueBinding::Type(def));
        }
        None
    }

    fn find_local(&self, name: &str) -> Option<(usize, LocalId)> {
        for (index, frame) in self.frames.iter().enumerate().rev() {
            for scope in frame.blocks.iter().rev() {
                if let Some(&id) = scope.get(name) {
                    return Some((index, id));
                }
            }
        }
        None
    }

    fn lookup_member(&self, owner: DefId, name: &str) -> Option<ValueBinding> {
        if let Some(structure) = self.context.def(owner).as_struct() {
            if let Some(index) = structure.field_index(name) {
                return Some(ValueBinding::Field {
                    owner,
                    index: index as u32,
                });
            }
        }
        self.context
            .find_method(owner, name)
            .map(ValueBinding::Method)
    }

    fn mark_this_captured(&mut self) {
        for frame in &mut self.frames {
            if frame.closure.is_some() {
                frame.captures_this = true;
            }
        }
    }

    fn note_constant_dependency(&mut self, referenced: GlobalConstId) {
        let Some(current) = self.current_const else {
            return;
        };
        if current == referenced {
            return;
        }
        self.const_dependencies
            .entry(current)
            .or_default()
            .push(referenced);
    }

    fn record_value(&mut self, node: NodeId, binding: ValueBinding) {
        self.values.insert(node, binding);
    }

    fn resolve_identifier(&mut self, node: NodeId, name: &Ident) {
        match self.lookup_value(name) {
            Some(binding) => self.record_value(node, binding),
            None => {
                let mut error = CompileError::at(
                    ErrorCode::UnknownIdentifier,
                    name.span,
                    format!("cannot find `{}` in scope", name.name),
                );
                if let Some(suggestion) = self.suggest_value_name(&name.name) {
                    error = error.with_help(format!("a name `{suggestion}` is in scope"));
                }
                self.report(error);
            }
        }
    }

    fn suggest_value_name(&self, name: &str) -> Option<String> {
        let mut candidates: Vec<String> = Vec::new();
        for frame in &self.frames {
            for scope in &frame.blocks {
                candidates.extend(scope.keys().cloned());
            }
        }
        let module = &self.modules[self.module.index()];
        candidates.extend(module.functions.keys().cloned());
        candidates.extend(module.constants.keys().cloned());
        candidates.extend(module.types.keys().cloned());
        closest(name, candidates.iter().map(|entry| entry.as_str()))
    }

    // canh bao bien khai bao ma khong doc lan nao. Tham so ham cung tinh,
    // dat ten `_x` la im ngay.
    fn report_unused_locals(&mut self) {
        let mut warnings = Vec::new();
        for local in &self.locals {
            let reportable = matches!(local.origin, LocalOrigin::Let | LocalOrigin::Const);
            let da_dung = self.used_locals.borrow().contains(&local.id);
            if !reportable || local.name.starts_with('_') || da_dung {
                continue;
            }
            warnings.push(
                CompileError::warning(
                    ErrorCode::UnusedLocal,
                    local.span,
                    format!("`{}` is never read", local.name),
                )
                .with_help(format!("rename it `_{}` to silence this", local.name)),
            );
        }
        for warning in warnings {
            self.report(warning);
        }
    }

    fn report_unused_imports(&mut self) {
        let mut errors = Vec::new();
        for module in &self.modules {
            for (name, binding) in &module.imports {
                if binding.used {
                    continue;
                }
                errors.push(
                    CompileError::at(
                        ErrorCode::UnusedImport,
                        binding.span,
                        format!("`{name}` is imported but never used"),
                    )
                    .with_help("remove the import"),
                );
            }
        }
        errors.sort_by_key(|error| (error.span().file.0, error.span().start));
        for error in errors {
            self.report(error);
        }
    }
}

// statement, bieu thuc va pattern

impl Resolver<'_> {
    fn resolve_block(&mut self, block: &Block) {
        self.push_scope();
        for statement in &block.statements {
            self.resolve_stmt(statement);
        }
        self.pop_scope();
    }

    fn resolve_stmt(&mut self, statement: &Stmt) {
        match &statement.kind {
            StmtKind::Let(decl) => self.resolve_binding(
                decl.pattern.clone(),
                decl.ty.as_ref(),
                &decl.value,
                LocalOrigin::Let,
            ),
            StmtKind::Const(decl) => self.resolve_binding(
                decl.pattern.clone(),
                decl.ty.as_ref(),
                &decl.value,
                LocalOrigin::Const,
            ),
            StmtKind::Assign(assign) => {
                self.resolve_expr(&assign.target);
                self.resolve_expr(&assign.value);
            }
            StmtKind::Expr(expr) => self.resolve_expr(expr),
            StmtKind::If(statement) => self.resolve_if(statement),
            StmtKind::While(statement) => {
                self.resolve_expr(&statement.condition);
                self.enter_loop();
                self.resolve_block(&statement.body);
                self.leave_loop();
            }
            StmtKind::For(statement) => {
                self.resolve_expr(&statement.iterable);
                self.push_scope();
                self.declare_irrefutable(&statement.pattern, LocalOrigin::LoopBinding, false);
                self.enter_loop();
                self.resolve_block(&statement.body);
                self.leave_loop();
                self.pop_scope();
            }
            StmtKind::Match(statement) => {
                self.resolve_expr(&statement.scrutinee);
                for arm in &statement.arms {
                    self.push_scope();
                    self.resolve_pattern(&arm.pattern);
                    if let Some(guard) = &arm.guard {
                        self.resolve_expr(guard);
                    }
                    match &arm.body {
                        MatchArmBody::Block(block) => self.resolve_block(block),
                        MatchArmBody::Stmt(inner) => self.resolve_stmt(inner),
                    }
                    self.pop_scope();
                }
            }
            StmtKind::Return(value) => {
                if let Some(value) = value {
                    self.resolve_expr(value);
                }
            }
            StmtKind::Fail(value) => self.resolve_expr(value),
            StmtKind::Break => {
                self.check_jump(statement.span, ErrorCode::BreakOutsideLoop, "break")
            }
            StmtKind::Continue => {
                self.check_jump(statement.span, ErrorCode::ContinueOutsideLoop, "continue")
            }
            StmtKind::Block(block) => self.resolve_block(block),
        }
    }

    fn resolve_binding(
        &mut self,
        pattern: IrrefutablePattern,
        annotation: Option<&TypeExpr>,
        value: &Expr,
        origin: LocalOrigin,
    ) {
        if let Some(annotation) = annotation {
            self.resolve_type(annotation, TypePosition::Value);
        }
        // gia tri khoi tao duoc resolve truoc: mot binding chi nhin thay duoc
        // sau chinh khai bao cua no (16.1 buoc 1)
        let names = collect_pattern_bindings(&pattern, &mut Vec::new());
        let pending: Vec<(String, Span)> = names
            .iter()
            .map(|(name, _)| (name.name.clone(), name.span))
            .collect();
        let previous = std::mem::replace(&mut self.pending_bindings, pending);
        self.resolve_expr(value);
        self.pending_bindings = previous;

        self.declare_irrefutable(&pattern, origin, origin == LocalOrigin::Let);
    }

    fn resolve_if(&mut self, statement: &crate::ast::IfStmt) {
        self.resolve_expr(&statement.condition);
        self.resolve_block(&statement.then_block);
        match &statement.else_branch {
            Some(crate::ast::ElseBranch::If(nested)) => self.resolve_if(nested),
            Some(crate::ast::ElseBranch::Block(block)) => self.resolve_block(block),
            None => {}
        }
    }

    fn enter_loop(&mut self) {
        if let Some(frame) = self.frames.last_mut() {
            frame.loop_depth += 1;
        }
    }

    fn leave_loop(&mut self) {
        if let Some(frame) = self.frames.last_mut() {
            frame.loop_depth = frame.loop_depth.saturating_sub(1);
        }
    }

    fn check_jump(&mut self, span: Span, code: ErrorCode, keyword: &str) {
        let inside = self.frames.last().is_some_and(|frame| frame.loop_depth > 0);
        if inside {
            return;
        }
        let in_closure = self
            .frames
            .last()
            .is_some_and(|frame| frame.closure.is_some());
        let mut error = CompileError::at(
            code,
            span,
            format!("`{keyword}` is not inside a `while` or `for` in this function"),
        );
        if in_closure {
            error = error.with_note(
                "a closure body is a function boundary, so it cannot leave an enclosing loop \
                 (grammar 13.5.4)",
            );
        }
        self.report(error);
    }

    fn declare_irrefutable(
        &mut self,
        pattern: &IrrefutablePattern,
        origin: LocalOrigin,
        reassignable: bool,
    ) {
        match &pattern.kind {
            IrrefutablePatternKind::Binding(name) => {
                self.declare_local(name, origin, reassignable);
            }
            IrrefutablePatternKind::Wildcard => {}
            IrrefutablePatternKind::Tuple(elements) => {
                for element in elements {
                    self.declare_irrefutable(element, origin, reassignable);
                }
            }
        }
    }

    fn resolve_pattern(&mut self, pattern: &Pattern) {
        match &pattern.kind {
            PatternKind::Wildcard
            | PatternKind::Null
            | PatternKind::Bool(_)
            | PatternKind::Int { .. }
            | PatternKind::Char(_)
            | PatternKind::Str(_)
            | PatternKind::Range { .. } => {}
            PatternKind::Binding(name) => {
                self.declare_local(name, LocalOrigin::PatternBinding, false);
            }
            PatternKind::Variant {
                enum_name,
                variant,
                payload,
            } => {
                self.resolve_pattern_type(pattern.id, enum_name);
                let _ = variant;
                if let Some(payload) = payload {
                    for element in payload {
                        self.resolve_pattern(element);
                    }
                }
            }
            PatternKind::Struct { name, fields, .. } => {
                self.resolve_pattern_type(pattern.id, name);
                for field in fields {
                    match &field.pattern {
                        Some(inner) => self.resolve_pattern(inner),
                        // dang viet tat buoc mot bien trung ten truong
                        None => {
                            self.declare_local(&field.name, LocalOrigin::PatternBinding, false);
                        }
                    }
                }
            }
            PatternKind::Tuple(elements) => {
                for element in elements {
                    self.resolve_pattern(element);
                }
            }
            PatternKind::Or(alternatives) => {
                // moi nhanh deu buoc dung nhung ten do, nen khai bao mot lan
                // thoi de cai check trung ten cua block con co nghia
                for (index, alternative) in alternatives.iter().enumerate() {
                    if index == 0 {
                        self.resolve_pattern(alternative);
                    } else {
                        self.resolve_repeated_alternative(alternative);
                    }
                }
            }
        }
    }

    fn resolve_repeated_alternative(&mut self, pattern: &Pattern) {
        match &pattern.kind {
            PatternKind::Binding(name) => {
                let existing = self
                    .frames
                    .last()
                    .and_then(|frame| frame.blocks.last())
                    .and_then(|scope| scope.get(&name.name).copied());
                match existing {
                    Some(id) => {
                        self.declared_locals.insert(name.span, id);
                    }
                    None => {
                        self.declare_local(name, LocalOrigin::PatternBinding, false);
                    }
                }
            }
            PatternKind::Variant {
                enum_name, payload, ..
            } => {
                self.resolve_pattern_type(pattern.id, enum_name);
                if let Some(payload) = payload {
                    for element in payload {
                        self.resolve_repeated_alternative(element);
                    }
                }
            }
            PatternKind::Struct { name, fields, .. } => {
                self.resolve_pattern_type(pattern.id, name);
                for field in fields {
                    match &field.pattern {
                        Some(inner) => self.resolve_repeated_alternative(inner),
                        None => {
                            let existing = self
                                .frames
                                .last()
                                .and_then(|frame| frame.blocks.last())
                                .and_then(|scope| scope.get(&field.name.name).copied());
                            match existing {
                                Some(id) => {
                                    self.declared_locals.insert(field.name.span, id);
                                }
                                None => {
                                    self.declare_local(
                                        &field.name,
                                        LocalOrigin::PatternBinding,
                                        false,
                                    );
                                }
                            }
                        }
                    }
                }
            }
            PatternKind::Tuple(elements) => {
                for element in elements {
                    self.resolve_repeated_alternative(element);
                }
            }
            PatternKind::Or(alternatives) => {
                for alternative in alternatives {
                    self.resolve_repeated_alternative(alternative);
                }
            }
            _ => {}
        }
    }

    fn resolve_pattern_type(&mut self, node: NodeId, name: &Ident) {
        let path = TypePath {
            module: None,
            name: name.clone(),
            span: name.span,
        };
        match self.lookup_type_path(&path) {
            Some(def) => {
                self.check_type_visibility(def, name.span);
                self.pattern_defs.insert(node, def);
            }
            None => {
                let mut error = CompileError::at(
                    ErrorCode::UnknownType,
                    name.span,
                    format!("cannot find type `{}` in scope", name.name),
                );
                if let Some(suggestion) = self.suggest_type_name(&name.name) {
                    error = error.with_help(format!("a type named `{suggestion}` is in scope"));
                }
                self.report(error);
            }
        }
    }

    fn resolve_expr(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Char(_)
            | ExprKind::Bool(_)
            | ExprKind::Null => {}
            ExprKind::Str(literal) => {
                for part in &literal.parts {
                    if let StringPart::Interp(inner) = part {
                        self.resolve_expr(inner);
                    }
                }
            }
            ExprKind::This => self.resolve_this(expr.span),
            ExprKind::Ident(name) => {
                if self.is_pending_binding(&name.name) && self.find_local(&name.name).is_none() {
                    let declared = self
                        .pending_bindings
                        .iter()
                        .find(|(pending, _)| *pending == name.name)
                        .map(|(_, span)| *span);
                    let mut error = CompileError::at(
                        ErrorCode::SelfReferentialClosure,
                        name.span,
                        format!(
                            "`{}` is not initialised until this declaration finishes",
                            name.name
                        ),
                    )
                    .with_help("recursion needs a named top-level function");
                    if let Some(declared) = declared {
                        error = error.with_secondary(declared, "being declared here");
                    }
                    self.report(error);
                    return;
                }
                self.resolve_identifier(expr.id, name);
            }
            ExprKind::Array(elements) | ExprKind::Set(elements) | ExprKind::Tuple(elements) => {
                for element in elements {
                    self.resolve_expr(element);
                }
            }
            ExprKind::Map(entries) => {
                for entry in entries {
                    self.resolve_expr(&entry.key);
                    self.resolve_expr(&entry.value);
                }
            }
            ExprKind::Group(inner) => self.resolve_expr(inner),
            ExprKind::Closure(closure) => self.resolve_closure(closure),
            ExprKind::Unary { operand, .. } => self.resolve_expr(operand),
            ExprKind::Binary { lhs, rhs, .. } => {
                self.resolve_expr(lhs);
                self.resolve_expr(rhs);
            }
            ExprKind::Range { start, end, .. } => {
                self.resolve_expr(start);
                self.resolve_expr(end);
            }
            ExprKind::Catch { operand, handler } => {
                self.resolve_expr(operand);
                match handler {
                    CatchHandler::Discard(block) => self.resolve_block(block),
                    CatchHandler::Bind { name, block } => {
                        self.push_scope();
                        self.declare_local(name, LocalOrigin::CatchBinding, false);
                        self.resolve_block(block);
                        self.pop_scope();
                    }
                    CatchHandler::Value(value) => self.resolve_expr(value),
                }
            }
            ExprKind::Field { base, name } => {
                self.resolve_expr(base);
                self.resolve_qualified_member(expr.id, base, name);
            }
            ExprKind::TupleField { base, .. } => self.resolve_expr(base),
            ExprKind::Call { callee, args } => {
                self.resolve_expr(callee);
                for Argument { value, .. } in args {
                    self.resolve_expr(value);
                }
            }
            ExprKind::Index { base, index } => {
                self.resolve_expr(base);
                self.resolve_expr(index);
            }
            ExprKind::NullPropagate(inner) | ExprKind::ErrorPropagate(inner) => {
                self.resolve_expr(inner)
            }
            ExprKind::TypeArgs { base, args } => {
                self.resolve_expr(base);
                for arg in args {
                    self.resolve_type(arg, TypePosition::Value);
                }
            }
            ExprKind::StructLit(literal) => {
                match self.lookup_type_path(&literal.path) {
                    Some(def) => {
                        self.check_type_visibility(def, literal.path.span);
                        self.record_value(expr.id, ValueBinding::Type(def));
                    }
                    None => {
                        let mut error = CompileError::at(
                            ErrorCode::UnknownType,
                            literal.path.span,
                            format!(
                                "cannot find type `{}` in scope",
                                render_type_path(&literal.path)
                            ),
                        );
                        if let Some(suggestion) = self.suggest_type_name(&literal.path.name.name) {
                            error =
                                error.with_help(format!("a type named `{suggestion}` is in scope"));
                        }
                        self.report(error);
                    }
                }
                for arg in &literal.type_args {
                    self.resolve_type(arg, TypePosition::Value);
                }
                for field in &literal.fields {
                    self.resolve_expr(&field.value);
                }
            }
        }
    }

    fn resolve_this(&mut self, span: Span) {
        let owner = self.frames.last().and_then(|frame| frame.this_owner);
        if owner.is_none() {
            self.report(
                CompileError::at(
                    ErrorCode::ThisOutsideMethod,
                    span,
                    "`this` is only bound inside a method body",
                )
                .with_note("closures inside a method may use it too (grammar 16.5)"),
            );
            return;
        }
        self.mark_this_captured();
    }

    fn resolve_qualified_member(&mut self, node: NodeId, base: &Expr, name: &Ident) {
        let Some(ValueBinding::Module(target)) = self.values.get(&base.id).copied() else {
            return;
        };
        let info = &self.modules[target.index()];
        let binding = if let Some(&global) = info.constants.get(&name.name) {
            let visibility = self.globals[global.index()].visibility;
            self.check_member_visibility(visibility, target, name, "constant");
            self.note_constant_dependency(global);
            Some(ValueBinding::GlobalConst(global))
        } else if let Some(&func) = info.functions.get(&name.name) {
            let visibility = self.context.func(func).visibility;
            self.check_member_visibility(visibility, target, name, "function");
            Some(ValueBinding::Function(func))
        } else if let Some(&def) = info.types.get(&name.name) {
            let visibility = self.context.def(def).visibility;
            self.check_member_visibility(visibility, target, name, "type");
            Some(ValueBinding::Type(def))
        } else {
            None
        };

        match binding {
            Some(binding) => self.record_value(node, binding),
            None => {
                let module = self.modules[target.index()].path.join("\\");
                let candidates: Vec<String> = self.modules[target.index()]
                    .functions
                    .keys()
                    .chain(self.modules[target.index()].constants.keys())
                    .chain(self.modules[target.index()].types.keys())
                    .cloned()
                    .collect();
                let mut error = CompileError::at(
                    ErrorCode::UnknownIdentifier,
                    name.span,
                    format!("module `{module}` has no member `{}`", name.name),
                );
                if let Some(suggestion) =
                    closest(&name.name, candidates.iter().map(|entry| entry.as_str()))
                {
                    error = error.with_help(format!("did you mean `{suggestion}`?"));
                }
                self.report(error);
            }
        }
    }

    fn check_member_visibility(
        &mut self,
        visibility: VisibilityKind,
        owner: ModuleId,
        name: &Ident,
        what: &str,
    ) {
        if visibility == VisibilityKind::Public || owner == self.module {
            return;
        }
        let module = self.modules[owner.index()].path.join("\\");
        self.report(
            CompileError::at(
                ErrorCode::PrivateAccess,
                name.span,
                format!("the {what} `{}` is private to module `{module}`", name.name),
            )
            .with_help(format!("declare it `pub` in `{module}`")),
        );
    }

    fn resolve_closure(&mut self, closure: &ClosureExpr) {
        let this_owner = self.frames.last().and_then(|frame| frame.this_owner);
        self.frames.push(Frame::new(this_owner, Some(closure.id)));

        for param in &closure.params {
            self.resolve_type(&param.ty, TypePosition::Value);
            self.declare_parameter(param);
        }
        if let Some(ret) = &closure.return_type {
            self.resolve_type(ret, TypePosition::Return);
        }
        self.resolve_block(&closure.body);

        let frame = self
            .frames
            .pop()
            .expect("the closure frame was just pushed");
        self.closures.insert(
            closure.id,
            ClosureInfo {
                captures: frame.captures,
                captures_this: frame.captures_this,
            },
        );
    }

    fn is_pending_binding(&self, name: &str) -> bool {
        self.pending_bindings
            .iter()
            .any(|(pending, _)| pending == name)
    }
}

// pass 5: constant initialisation order and the entry point

impl Resolver<'_> {
    fn order_module_constants(&mut self) {
        #[derive(Clone, Copy, PartialEq)]
        enum Mark {
            Unvisited,
            InProgress,
            Done,
        }

        let count = self.globals.len();
        let mut marks = vec![Mark::Unvisited; count];
        let mut order = Vec::with_capacity(count);
        let mut stack: Vec<(GlobalConstId, usize)> = Vec::new();

        for start in 0..count {
            if marks[start] != Mark::Unvisited {
                continue;
            }
            stack.push((GlobalConstId(start as u32), 0));
            marks[start] = Mark::InProgress;

            while let Some((current, next)) = stack.pop() {
                let dependencies = self
                    .const_dependencies
                    .get(&current)
                    .cloned()
                    .unwrap_or_default();
                if next < dependencies.len() {
                    stack.push((current, next + 1));
                    let dependency = dependencies[next];
                    match marks[dependency.index()] {
                        Mark::Unvisited => {
                            marks[dependency.index()] = Mark::InProgress;
                            stack.push((dependency, 0));
                        }
                        Mark::InProgress => {
                            let entry = &self.globals[dependency.index()];
                            let name = entry.name.clone();
                            let span = entry.span;
                            self.report(
                                CompileError::at(
                                    ErrorCode::CircularConstInitialisation,
                                    self.globals[current.index()].span,
                                    format!(
                                        "`{}` depends on `{name}`, which depends on it in turn",
                                        self.globals[current.index()].name
                                    ),
                                )
                                .with_secondary(span, "the other constant in the cycle"),
                            );
                            marks[dependency.index()] = Mark::Done;
                        }
                        Mark::Done => {}
                    }
                } else {
                    marks[current.index()] = Mark::Done;
                    order.push(current);
                }
            }
        }

        self.const_init_order = order;
    }

    fn check_entry_point(&mut self) {
        let root = self.root_module;
        let Some(&main) = self.modules[root.index()].functions.get("main") else {
            let span = self.units[root.index()].span;
            self.report(
                CompileError::at(
                    ErrorCode::MissingMain,
                    span,
                    "this module is the entry file but declares no `fn main()`",
                )
                .with_help("add `fn main() { ... }`"),
            );
            return;
        };

        let definition = self.context.func(main).clone();
        let mut valid = true;
        if !definition.params.is_empty() {
            self.report(
                CompileError::at(
                    ErrorCode::InvalidMainSignature,
                    definition.params[0].span,
                    "`main` takes no parameters",
                )
                .with_help("read the command line from the stdlib instead"),
            );
            valid = false;
        }
        if definition.is_generic() {
            self.report(CompileError::at(
                ErrorCode::InvalidMainSignature,
                definition.span,
                "`main` may not be generic",
            ));
            valid = false;
        }
        if !matches!(definition.ret, TypeId::VOID | TypeId::INT) {
            let shown = self.context.display(definition.ret);
            self.report(
                CompileError::at(
                    ErrorCode::InvalidMainSignature,
                    definition.span,
                    format!("`main` cannot return `{shown}`"),
                )
                .with_help("the legal forms are `main()`, `main(): void!`, `main(): int` and `main(): int!`"),
            );
            valid = false;
        }
        if valid {
            self.entry = Some(main);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{ElseBranch, IfStmt};
    use crate::Session;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Project {
        root: PathBuf,
    }

    impl Project {
        fn new(files: &[(&str, &str)]) -> Project {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let mut root = std::env::temp_dir();
            root.push(format!("pump-resolve-{}-{unique}", std::process::id()));
            std::fs::create_dir_all(&root).expect("the scratch directory is writable");

            for (name, text) in files {
                let path = root.join(name);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).expect("the scratch directory is writable");
                }
                std::fs::write(&path, text).expect("the scratch directory is writable");
            }
            Project { root }
        }
    }

    impl Drop for Project {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn resolve_entry(root: &Path, source: &str) -> (Option<Resolution>, Diagnostics) {
        let mut session = Session::new();
        let file = session
            .sources
            .add(root.join("main.pump"), source.to_string());
        let text = source.to_string();

        let tokens = crate::lexer::tokenize(file, &text, &mut session.diagnostics);
        let unit = crate::parser::parse(
            file,
            vec!["main".to_string()],
            &tokens,
            &mut session.node_ids,
            &mut session.diagnostics,
        );

        let mut collected = Diagnostics::new();
        let resolution = match resolve(vec![unit], root, &mut session, &mut collected) {
            Ok(resolution) => Some(resolution),
            Err(error) => {
                collected.push(error);
                None
            }
        };

        let mut all = std::mem::take(&mut session.diagnostics);
        all.extend(collected);
        (resolution, all)
    }

    fn diagnose(source: &str) -> Diagnostics {
        resolve_entry(Path::new("."), source).1
    }

    fn error_codes(diagnostics: &Diagnostics) -> Vec<ErrorCode> {
        diagnostics
            .entries()
            .iter()
            .filter(|entry| entry.is_error())
            .map(|entry| entry.code)
            .collect()
    }

    #[track_caller]
    fn assert_reports(source: &str, code: ErrorCode) {
        let found = error_codes(&diagnose(source));
        assert!(
            found.contains(&code),
            "expected {code:?}, found {found:?}\n--- source ---\n{source}"
        );
    }

    #[track_caller]
    fn assert_clean(source: &str) {
        let found = error_codes(&diagnose(source));
        assert!(
            found.is_empty(),
            "expected no errors, found {found:?}\n--- source ---\n{source}"
        );
    }

    #[track_caller]
    fn assert_project_clean(project: &Project, source: &str) {
        let (_, diagnostics) = resolve_entry(&project.root, source);
        let found = error_codes(&diagnostics);
        assert!(
            found.is_empty(),
            "expected no errors, found {found:?}\n--- source ---\n{source}"
        );
    }

    #[track_caller]
    fn assert_project_reports(project: &Project, source: &str, code: ErrorCode) {
        let (_, diagnostics) = resolve_entry(&project.root, source);
        let found = error_codes(&diagnostics);
        assert!(
            found.contains(&code),
            "expected {code:?}, found {found:?}\n--- source ---\n{source}"
        );
    }

    fn resolution_of(source: &str) -> Resolution {
        let (resolution, diagnostics) = resolve_entry(Path::new("."), source);
        let found = error_codes(&diagnostics);
        assert!(found.is_empty(), "expected no errors, found {found:?}");
        resolution.expect("the entry unit always resolves")
    }

    #[test]
    fn a_sibling_module_is_loaded_and_its_pub_items_are_reachable() {
        let project = Project::new(&[("util.pump", "pub fn helper(): int {\n    return 1\n}\n")]);
        assert_project_clean(
            &project,
            "import util\n\nfn main() {\n    println(util.helper())\n}\n",
        );
    }

    #[test]
    fn a_nested_import_path_maps_to_a_directory() {
        let project = Project::new(&[(
            "net/http.pump",
            "pub fn get(url: string): string {\n    return url\n}\n",
        )]);
        assert_project_clean(
            &project,
            "import net\\http\n\nfn main() {\n    println(http.get(\"x\"))\n}\n",
        );
    }

    #[test]
    fn an_import_alias_binds_the_alias_and_not_the_last_segment() {
        let project = Project::new(&[(
            "net/http.pump",
            "pub fn get(url: string): string {\n    return url\n}\n",
        )]);
        assert_project_clean(
            &project,
            "import net\\http as web\n\nfn main() {\n    println(web.get(\"x\"))\n}\n",
        );
        assert_project_reports(
            &project,
            "import net\\http as web\n\nfn main() {\n    println(http.get(\"x\"))\n}\n",
            ErrorCode::UnknownIdentifier,
        );
    }

    #[test]
    fn a_missing_module_is_reported() {
        let project = Project::new(&[]);
        assert_project_reports(
            &project,
            "import util\n\nfn main() {\n    println(util.helper())\n}\n",
            ErrorCode::ModuleNotFound,
        );
    }

    #[test]
    fn an_import_cycle_is_reported() {
        let project = Project::new(&[(
            "util.pump",
            "import main\n\npub fn helper(): int {\n    return main.seed()\n}\n",
        )]);
        assert_project_reports(
            &project,
            "import util\n\npub fn seed(): int {\n    return 1\n}\n\nfn main() {\n    println(util.helper())\n}\n",
            ErrorCode::CircularImport,
        );
    }

    #[test]
    fn an_unused_import_is_an_error() {
        let project = Project::new(&[("util.pump", "pub fn helper(): int {\n    return 1\n}\n")]);
        assert_project_reports(
            &project,
            "import util\n\nfn main() {\n}\n",
            ErrorCode::UnusedImport,
        );
    }

    #[test]
    fn a_private_item_is_not_reachable_from_another_module() {
        let project = Project::new(&[("util.pump", "fn helper(): int {\n    return 1\n}\n")]);
        assert_project_reports(
            &project,
            "import util\n\nfn main() {\n    println(util.helper())\n}\n",
            ErrorCode::PrivateAccess,
        );
    }

    #[test]
    fn a_module_member_that_does_not_exist_is_reported() {
        let project = Project::new(&[("util.pump", "pub fn helper(): int {\n    return 1\n}\n")]);
        assert_project_reports(
            &project,
            "import util\n\nfn main() {\n    println(util.missing())\n}\n",
            ErrorCode::UnknownIdentifier,
        );
    }

    #[test]
    fn top_level_declarations_are_order_independent() {
        assert_clean(
            "fn main() {\n    println(later())\n}\n\nfn later(): int {\n    return 1\n}\n",
        );
    }

    #[test]
    fn two_declarations_of_one_name_collide() {
        assert_reports(
            "fn f() {\n}\n\nfn f() {\n}\n\nfn main() {\n    f()\n}\n",
            ErrorCode::DuplicateDeclaration,
        );
    }

    #[test]
    fn a_duplicate_field_is_reported() {
        assert_reports(
            "struct User {\n    name: string\n    name: int\n}\n\nfn main() {\n}\n",
            ErrorCode::DuplicateField,
        );
    }

    #[test]
    fn a_field_and_a_method_may_not_share_a_name() {
        let source = "\
struct User {
    greet: string

    fn greet() {
    }
}

fn main() {
}
";
        let found = error_codes(&diagnose(source));
        assert!(
            found.contains(&ErrorCode::DuplicateMethod)
                || found.contains(&ErrorCode::DuplicateField),
            "expected a name collision, found {found:?}"
        );
    }

    #[test]
    fn duplicate_parameters_are_reported() {
        assert_reports(
            "fn f(a: int, a: int) {\n}\n\nfn main() {\n    f(1, 2)\n}\n",
            ErrorCode::DuplicateParameter,
        );
    }

    #[test]
    fn duplicate_generic_parameters_are_reported() {
        assert_reports(
            "fn f<T, T>(a: T) {\n}\n\nfn main() {\n}\n",
            ErrorCode::DuplicateGenericParameter,
        );
    }

    #[test]
    fn an_unknown_name_is_reported() {
        assert_reports(
            "fn main() {\n    println(nowhere)\n}\n",
            ErrorCode::UnknownIdentifier,
        );
    }

    #[test]
    fn an_unknown_type_is_reported() {
        assert_reports(
            "fn main() {\n    let x: Nowhere = 1\n    println(x)\n}\n",
            ErrorCode::UnknownType,
        );
    }

    #[test]
    fn a_predeclared_type_name_cannot_be_shadowed() {
        assert_reports(
            "fn main() {\n    let int = 3\n    println(int)\n}\n",
            ErrorCode::ShadowsPredeclaredName,
        );
    }

    #[test]
    fn a_prelude_value_may_be_shadowed() {
        assert_clean("fn main() {\n    let len = 3\n    println(len)\n}\n");
    }

    #[test]
    fn a_field_resolves_with_no_prefix_inside_a_method() {
        let source = "\
struct User {
    name: string

    fn greet() {
        println(name)
    }
}

fn main() {
    User { name: \"Minh\" }.greet()
}
";
        let resolution = resolution_of(source);
        let bindings = bindings_in_method(&resolution, "User", "greet");
        assert_eq!(
            bindings
                .iter()
                .filter(|(name, _)| name == "name")
                .map(|(_, binding)| *binding)
                .collect::<Vec<_>>(),
            vec![ValueBinding::Field {
                owner: DefId(3),
                index: 0
            }],
            "an unqualified field name resolves at step 4 of the scope walk"
        );
    }

    #[test]
    fn a_parameter_shadows_a_field_and_this_reaches_the_field() {
        let source = "\
struct User {
    name: string

    fn rename(name: string) {
        this.name = name
    }
}

fn main() {
    User { name: \"Minh\" }.rename(\"Linh\")
}
";
        let resolution = resolution_of(source);
        let bindings = bindings_in_method(&resolution, "User", "rename");
        let mentions: Vec<ValueBinding> = bindings
            .iter()
            .filter(|(name, _)| name == "name")
            .map(|(_, binding)| *binding)
            .collect();
        // `this.name` la bieu thuc truong, nen `name` tran o day chi co the
        // la ve phai, va no trung tham so o buoc 2
        assert_eq!(mentions.len(), 1, "found {mentions:?}");
        assert!(
            matches!(mentions[0], ValueBinding::Local(_)),
            "a parameter shadows a field of the same name; found {:?}",
            mentions[0]
        );
    }

    #[test]
    fn a_method_resolves_with_no_prefix_inside_another_method() {
        let source = "\
struct User {
    name: string

    fn greet() {
        println(name)
    }

    fn welcome() {
        greet()
    }
}

fn main() {
    User { name: \"Minh\" }.welcome()
}
";
        let resolution = resolution_of(source);
        let bindings = bindings_in_method(&resolution, "User", "welcome");
        assert!(
            bindings.iter().any(
                |(name, binding)| name == "greet" && matches!(binding, ValueBinding::Method(_))
            ),
            "an unqualified method name means `this.f()`; found {bindings:?}"
        );
    }

    #[test]
    fn this_outside_a_method_is_an_error() {
        assert_reports(
            "fn main() {\n    println(this)\n}\n",
            ErrorCode::ThisOutsideMethod,
        );
    }

    #[test]
    fn a_local_shadows_an_enclosing_block_but_not_its_own() {
        assert_clean(
            "fn main() {\n    let x = 1\n    {\n        let x = 2\n        println(x)\n    }\n    println(x)\n}\n",
        );
        assert_reports(
            "fn main() {\n    let x = 1\n    let x = 2\n    println(x)\n}\n",
            ErrorCode::DuplicateDeclaration,
        );
    }

    #[test]
    fn break_outside_a_loop_is_an_error() {
        assert_reports("fn main() {\n    break\n}\n", ErrorCode::BreakOutsideLoop);
        assert_reports(
            "fn main() {\n    continue\n}\n",
            ErrorCode::ContinueOutsideLoop,
        );
    }

    #[test]
    fn a_closure_body_is_a_loop_boundary() {
        let source = "\
fn main() {
    for i in 0..10 {
        let stop = fn() {
            break
        }
        stop()
    }
}
";
        assert_reports(source, ErrorCode::BreakOutsideLoop);
    }

    #[test]
    fn a_closure_records_what_it_captures() {
        let source = "\
fn main() {
    let base = 10
    let bump = fn(n: int): int {
        return n + base
    }
    println(bump(1))
}
";
        let resolution = resolution_of(source);
        assert_eq!(resolution.closures.len(), 1);
        let info = resolution
            .closures
            .values()
            .next()
            .expect("one closure was resolved");
        assert_eq!(info.captures.len(), 1, "the closure captures `base`");
        assert!(!info.captures_this);
        let captured = resolution.local(info.captures[0]);
        assert_eq!(captured.name, "base");
        assert!(captured.captured, "a captured binding is marked as such");
    }

    #[test]
    fn a_closure_cannot_refer_to_the_binding_it_initialises() {
        assert_reports(
            "fn main() {\n    let f = fn() {\n        f()\n    }\n    f()\n}\n",
            ErrorCode::SelfReferentialClosure,
        );
    }

    #[test]
    fn a_closure_inside_a_method_may_capture_this() {
        let source = "\
struct Counter {
    total: int

    fn bump() {
        let step = fn(): int {
            return total
        }
        println(step())
    }
}

fn main() {
    Counter { total: 1 }.bump()
}
";
        let resolution = resolution_of(source);
        let info = resolution
            .closures
            .values()
            .next()
            .expect("one closure was resolved");
        assert!(info.captures_this, "reaching a field reaches `this`");
    }

    #[test]
    fn module_constants_are_ordered_by_dependency() {
        let source = "\
const B: int = A + 1
const A: int = 1

fn main() {
    println(B)
}
";
        let resolution = resolution_of(source);
        let order: Vec<&str> = resolution
            .const_init_order
            .iter()
            .map(|id| resolution.globals[id.index()].name.as_str())
            .collect();
        assert_eq!(order, vec!["A", "B"]);
    }

    #[test]
    fn a_constant_cycle_is_reported() {
        assert_reports(
            "const A: int = B\nconst B: int = A\n\nfn main() {\n    println(A)\n}\n",
            ErrorCode::CircularConstInitialisation,
        );
    }

    #[test]
    fn a_missing_main_is_reported() {
        assert_reports("fn other() {\n}\n", ErrorCode::MissingMain);
    }

    #[test]
    fn main_may_not_take_parameters() {
        assert_reports(
            "fn main(argc: int) {\n    println(argc)\n}\n",
            ErrorCode::InvalidMainSignature,
        );
    }

    #[test]
    fn main_may_return_int() {
        assert_clean("fn main(): int {\n    return 0\n}\n");
    }

    #[test]
    fn the_entry_point_is_recorded() {
        let resolution = resolution_of("fn main() {\n}\n");
        let entry = resolution.entry.expect("`main` was found");
        assert_eq!(resolution.context.func(entry).name, "main");
    }

    #[test]
    fn an_unused_local_is_only_a_warning() {
        let diagnostics = diagnose("fn main() {\n    let unused = 1\n}\n");
        assert!(error_codes(&diagnostics).is_empty());
        assert!(
            diagnostics
                .entries()
                .iter()
                .any(|entry| entry.code == ErrorCode::UnusedLocal),
            "an unused local warns (grammar 16.9)"
        );
    }

    #[test]
    fn an_underscore_binding_never_warns() {
        let diagnostics = diagnose("fn main() {\n    let _ = 1\n}\n");
        assert!(diagnostics.entries().is_empty());
    }

    fn bindings_in_method(
        resolution: &Resolution,
        owner: &str,
        method: &str,
    ) -> Vec<(String, ValueBinding)> {
        let unit = &resolution.units[resolution.root_module.index()];
        for declaration in &unit.declarations {
            let Declaration::Struct(structure) = declaration else {
                continue;
            };
            if structure.name.name != owner {
                continue;
            }
            for member in &structure.members {
                let StructMember::Method(function) = member else {
                    continue;
                };
                if function.name.name != method {
                    continue;
                }
                let mut out = Vec::new();
                walk_block(&function.body, resolution, &mut out);
                return out;
            }
        }
        panic!("no method `{owner}.{method}` in the entry module");
    }

    fn walk_block(block: &Block, resolution: &Resolution, out: &mut Vec<(String, ValueBinding)>) {
        for statement in &block.statements {
            walk_stmt(statement, resolution, out);
        }
    }

    fn walk_stmt(statement: &Stmt, resolution: &Resolution, out: &mut Vec<(String, ValueBinding)>) {
        match &statement.kind {
            StmtKind::Let(declaration) => walk_expr(&declaration.value, resolution, out),
            StmtKind::Const(declaration) => walk_expr(&declaration.value, resolution, out),
            StmtKind::Assign(assign) => {
                walk_expr(&assign.target, resolution, out);
                walk_expr(&assign.value, resolution, out);
            }
            StmtKind::Expr(expr) => walk_expr(expr, resolution, out),
            StmtKind::If(inner) => walk_if(inner, resolution, out),
            StmtKind::While(inner) => {
                walk_expr(&inner.condition, resolution, out);
                walk_block(&inner.body, resolution, out);
            }
            StmtKind::For(inner) => {
                walk_expr(&inner.iterable, resolution, out);
                walk_block(&inner.body, resolution, out);
            }
            StmtKind::Match(inner) => {
                walk_expr(&inner.scrutinee, resolution, out);
                for arm in &inner.arms {
                    if let Some(guard) = &arm.guard {
                        walk_expr(guard, resolution, out);
                    }
                    match &arm.body {
                        MatchArmBody::Block(block) => walk_block(block, resolution, out),
                        MatchArmBody::Stmt(inner) => walk_stmt(inner, resolution, out),
                    }
                }
            }
            StmtKind::Return(value) => {
                if let Some(value) = value {
                    walk_expr(value, resolution, out);
                }
            }
            StmtKind::Fail(value) => walk_expr(value, resolution, out),
            StmtKind::Break | StmtKind::Continue => {}
            StmtKind::Block(block) => walk_block(block, resolution, out),
        }
    }

    fn walk_if(statement: &IfStmt, resolution: &Resolution, out: &mut Vec<(String, ValueBinding)>) {
        walk_expr(&statement.condition, resolution, out);
        walk_block(&statement.then_block, resolution, out);
        match &statement.else_branch {
            Some(ElseBranch::Block(block)) => walk_block(block, resolution, out),
            Some(ElseBranch::If(nested)) => walk_if(nested, resolution, out),
            None => {}
        }
    }

    fn walk_expr(expr: &Expr, resolution: &Resolution, out: &mut Vec<(String, ValueBinding)>) {
        match &expr.kind {
            ExprKind::Ident(name) => {
                if let Some(binding) = resolution.values.get(&expr.id) {
                    out.push((name.name.clone(), *binding));
                }
            }
            ExprKind::Group(inner)
            | ExprKind::Unary { operand: inner, .. }
            | ExprKind::NullPropagate(inner)
            | ExprKind::ErrorPropagate(inner)
            | ExprKind::TupleField { base: inner, .. }
            | ExprKind::TypeArgs { base: inner, .. } => walk_expr(inner, resolution, out),
            ExprKind::Field { base, .. } => walk_expr(base, resolution, out),
            ExprKind::Binary { lhs, rhs, .. } => {
                walk_expr(lhs, resolution, out);
                walk_expr(rhs, resolution, out);
            }
            ExprKind::Range { start, end, .. } => {
                walk_expr(start, resolution, out);
                walk_expr(end, resolution, out);
            }
            ExprKind::Index { base, index } => {
                walk_expr(base, resolution, out);
                walk_expr(index, resolution, out);
            }
            ExprKind::Call { callee, args } => {
                walk_expr(callee, resolution, out);
                for argument in args {
                    walk_expr(&argument.value, resolution, out);
                }
            }
            ExprKind::Array(elements) | ExprKind::Set(elements) | ExprKind::Tuple(elements) => {
                for element in elements {
                    walk_expr(element, resolution, out);
                }
            }
            ExprKind::Map(entries) => {
                for entry in entries {
                    walk_expr(&entry.key, resolution, out);
                    walk_expr(&entry.value, resolution, out);
                }
            }
            ExprKind::StructLit(literal) => {
                for field in &literal.fields {
                    walk_expr(&field.value, resolution, out);
                }
            }
            ExprKind::Str(literal) => {
                for part in &literal.parts {
                    if let StringPart::Interp(inner) = part {
                        walk_expr(inner, resolution, out);
                    }
                }
            }
            ExprKind::Closure(closure) => walk_block(&closure.body, resolution, out),
            ExprKind::Catch { operand, handler } => {
                walk_expr(operand, resolution, out);
                match handler {
                    CatchHandler::Discard(block) | CatchHandler::Bind { block, .. } => {
                        walk_block(block, resolution, out)
                    }
                    CatchHandler::Value(value) => walk_expr(value, resolution, out),
                }
            }
            _ => {}
        }
    }
}
