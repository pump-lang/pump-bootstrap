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

fn bundled2(segments: &[String]) -> Option<&'static str> {
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
    // &self.locals, borrow checker ...
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

    // luot 1:

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
        // println!("DBG: {:?} ->

        let file = if path.exists() {
            match session.load(&path) {
                Ok(file) => file,
                Err(error) => {
                    self.report(error);
                    return None;
                }
            }
        } else if let Some(source) = bundled2(segments) {
            // cai nay file trong project
            // dia that bai ...
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

    // luot 2: khai ...

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
                .with_note(format!( "the non-shadowable predeclared names are {}", NON_SHADOWABLE.join(", ") )),
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

// cai nay luot 3: tham

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
            self.functions.insert( id, FuncLocation { module, declaration: position, member: Some(member_index), }, );
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
                            CompileError::at( ErrorCode::NestedOptional, written.span, "an optional of an optional has no extra state to represent", )
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
        // ban truoc khi ...
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
                    // `void` chi hop ...
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
                format!("cannot find type `{}` in scope", re_type_path(path)),
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
