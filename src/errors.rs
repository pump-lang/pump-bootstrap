// bao loi. Ma loi, CompileError, va cai ham ve ra file:line:col voi may
// dau mui ten nho nho o duoi.
//
// trong compiler khong cho cho nao in thang ra man hinh. Pha nao co loi thi
// nan ra mot CompileError, nem vao Diagnostics, den cuoi driver moi in het
// mot the. Lam vay chay bao nhieu lan thu tu van y nhau.
//
// Ma loi bat dau tu E0417, moi pha giu mot khoang:
//
//   E0417 - E0449   lexer
//   E0450 - E0499   parser
//   E0500 - E0549   resolver
//   E0550 - E0649   type checker
//   E0650 - E0699   lower va sinh ma
//   E0700 - E0749   driver, linker, doc file

use std::fmt;
use std::path::{Path, PathBuf};

use crate::token::{FileId, Span};

/// Result type that every compiler routine which can fail gives back.
pub type CompileResult<T> = Result<T, CompileError>;

/// How bad a diagnostic is.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Severity {
    Warning,
    Error,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }
}

/// A span, maybe with a short message, drawn under the source line.
#[derive(Clone, Debug)]
pub struct Label {
    pub span: Span,
    pub message: Option<String>,
}

impl Label {
    pub fn new(span: Span) -> Label {
        Label {
            span,
            message: None,
        }
    }

    pub fn with_message(span: Span, message: impl Into<String>) -> Label {
        Label {
            span,
            message: Some(message.into()),
        }
    }
}

/// One diagnostic. No khong tu in ra duoc, phai qua render().
#[derive(Clone, Debug)]
pub struct CompileError {
    pub code: ErrorCode,
    pub severity: Severity,
    pub message: String,
    pub primary: Label,
    pub secondary: Vec<Label>,
    pub notes: Vec<String>,
    pub helps: Vec<String>,
}

impl CompileError {
    /// An error at span, message is just the title of the code.
    pub fn new(code: ErrorCode, span: Span) -> CompileError {
        CompileError {
            code,
            severity: Severity::Error,
            message: code.title().to_string(),
            primary: Label::new(span),
            secondary: Vec::new(),
            notes: Vec::new(),
            helps: Vec::new(),
        }
    }

    /// An error at span with a message written for this one place.
    pub fn at(code: ErrorCode, span: Span, message: impl Into<String>) -> CompileError {
        CompileError {
            code,
            severity: Severity::Error,
            message: message.into(),
            primary: Label::new(span),
            secondary: Vec::new(),
            notes: Vec::new(),
            helps: Vec::new(),
        }
    }

    /// A warning at span. Bien khai bao ma khong dung la ca dien hinh.
    pub fn warning(code: ErrorCode, span: Span, message: impl Into<String>) -> CompileError {
        CompileError {
            code,
            severity: Severity::Warning,
            message: message.into(),
            primary: Label::new(span),
            secondary: Vec::new(),
            notes: Vec::new(),
            helps: Vec::new(),
        }
    }

    /// Set the text put next to the caret.
    pub fn with_caret(mut self, message: impl Into<String>) -> CompileError {
        self.primary.message = Some(message.into());
        self
    }

    pub fn with_secondary(mut self, span: Span, message: impl Into<String>) -> CompileError {
        self.secondary.push(Label::with_message(span, message));
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> CompileError {
        self.notes.push(note.into());
        self
    }

    pub fn with_help(mut self, help: impl Into<String>) -> CompileError {
        self.helps.push(help.into());
        self
    }

    pub fn span(&self) -> Span {
        self.primary.span
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    /// Draw the diagnostic out using the source map.
    pub fn render(&self, sources: &SourceMap) -> String {
        render_diagnostic(self, sources)
    }
}

/// A pile of diagnostics collected so far.
#[derive(Clone, Debug, Default)]
pub struct Diagnostics {
    entries: Vec<CompileError>,
}

impl Diagnostics {
    pub fn new() -> Diagnostics {
        Diagnostics::default()
    }

    pub fn push(&mut self, diagnostic: CompileError) {
        self.entries.push(diagnostic);
    }

    /// Push the Err and give back None, or open up the Ok.
    pub fn absorb<T>(&mut self, result: CompileResult<T>) -> Option<T> {
        match result {
            Ok(value) => Some(value),
            Err(error) => {
                self.push(error);
                None
            }
        }
    }

    pub fn extend(&mut self, other: Diagnostics) {
        self.entries.extend(other.entries);
    }

    pub fn entries(&self) -> &[CompileError] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn has_errors(&self) -> bool {
        self.entries.iter().any(CompileError::is_error)
    }

    pub fn error_count(&self) -> usize {
        self.entries.iter().filter(|e| e.is_error()).count()
    }

    /// Sort by file then by byte offset, so the order printed does not
    /// depend on which pha happened to run first.
    pub fn sort(&mut self) {
        self.entries
            .sort_by_key(|e| (e.primary.span.file, e.primary.span.start, e.code.number()));
    }

    pub fn render(&self, sources: &SourceMap) -> String {
        let mut out = String::new();
        for entry in &self.entries {
            out.push_str(&entry.render(sources));
            out.push('\n');
        }
        out
    }
}

/// One source file: the path we show, and all of its text.
#[derive(Clone, Debug)]
pub struct SourceFile {
    pub id: FileId,
    pub path: PathBuf,
    pub text: String,
    line_starts: Vec<u32>,
}

impl SourceFile {
    /// Line number (dem tu 1) chua byte offset nay.
    pub fn line_of(&self, offset: u32) -> u32 {
        match self.line_starts.binary_search(&offset) {
            Ok(index) => index as u32 + 1,
            Err(index) => index as u32,
        }
    }

    /// Text of the line, khong keo theo dau xuong dong.
    pub fn line_text(&self, line: u32) -> &str {
        if line == 0 || line as usize > self.line_starts.len() {
            return "";
        }
        let start = self.line_starts[line as usize - 1] as usize;
        let end = self
            .line_starts
            .get(line as usize)
            .map(|&next| next as usize)
            .unwrap_or(self.text.len());
        self.text[start..end].trim_end_matches(['\n', '\r'])
    }

    pub fn line_count(&self) -> u32 {
        self.line_starts.len() as u32
    }
}

/// All source files this compile has read, tra theo FileId.
#[derive(Clone, Debug, Default)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    pub fn new() -> SourceMap {
        SourceMap::default()
    }

    /// Add a file, give back its id.
    pub fn add(&mut self, path: impl Into<PathBuf>, text: impl Into<String>) -> FileId {
        let text = text.into();
        let mut line_starts = vec![0u32];
        for (offset, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(offset as u32 + 1);
            }
        }
        let id = FileId(self.files.len() as u32);
        self.files.push(SourceFile {
            id,
            path: path.into(),
            text,
            line_starts,
        });
        id
    }

    pub fn get(&self, id: FileId) -> Option<&SourceFile> {
        self.files.get(id.index())
    }

    pub fn path(&self, id: FileId) -> &Path {
        static UNKNOWN: &str = "<unknown>";
        self.get(id)
            .map(|file| file.path.as_path())
            .unwrap_or_else(|| Path::new(UNKNOWN))
    }

    pub fn find_by_path(&self, path: &Path) -> Option<FileId> {
        self.files
            .iter()
            .find(|file| file.path == path)
            .map(|file| file.id)
    }

    pub fn files(&self) -> &[SourceFile] {
        &self.files
    }
}

fn render_diagnostic(diagnostic: &CompileError, sources: &SourceMap) -> String {
    let mut out = String::new();
    out.push_str(&format!( "{}[{}]: {}\n", diagnostic.severity.label(), diagnostic.code, diagnostic.message ));
    render_snippet(&mut out, &diagnostic.primary, sources, true);
    for label in &diagnostic.secondary {
        render_snippet(&mut out, label, sources, false);
    }
    for note in &diagnostic.notes {
        out.push_str(&format!("  = note: {note}\n"));
    }
    for help in &diagnostic.helps {
        out.push_str(&format!("  = help: {help}\n"));
    }
    out
}

fn render_snippet(out: &mut String, label: &Label, sources: &SourceMap, primary: bool) {
    let span = label.span;
    let arrow = if primary { "-->" } else { "..." };

    let Some(file) = sources.get(span.file) else {
        out.push_str(&format!(
            "  {arrow} <unknown>:{}:{}\n",
            span.line, span.column
        ));
        return;
    };

    let path = file.path.display();
    out.push_str(&format!("  {arrow} {path}:{}:{}\n", span.line, span.column));

    let line_text = file.line_text(span.line);
    let gutter_width = span.line.to_string().len();
    let blank_gutter = " ".repeat(gutter_width);

    out.push_str(&format!("{blank_gutter} |\n"));
    out.push_str(&format!("{} | {}\n", span.line, line_text));

    // day mui ten bi cat lai o dong dau tien cua span: span nhieu dong thi
    // chi gach tu cho bat dau den het dong do thoi. Gach het may dong nhin
    // roi mat.
    let column = span.column.max(1) as usize;
    let line_start = file
        .line_starts
        .get(span.line as usize - 1)
        .copied()
        .unwrap_or(0);
    let line_end = line_start + line_text.len() as u32;
    let underline_end = span.end.clamp(span.start, line_end);
    let width = (underline_end.saturating_sub(span.start) as usize).max(1);

    let pad = " ".repeat(column.saturating_sub(1));
    let carets = if primary { "^" } else { "-" }.repeat(width);
    match &label.message {
        Some(message) => {
            out.push_str(&format!("{blank_gutter} | {pad}{carets} {message}\n"));
        }
        None => out.push_str(&format!("{blank_gutter} | {pad}{carets}\n")),
    }
    out.push_str(&format!("{blank_gutter} |\n"));
}

/// Every code the compiler can print out.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[repr(u16)]
pub enum ErrorCode {
    // ---- lexer, E0417 - E0449 ----
    UnexpectedCharacter = 417,
    UnterminatedBlockComment = 418,
    UnterminatedString = 419,
    NewlineInString = 420,
    UnknownEscape = 421,
    InvalidUnicodeEscape = 422,
    AsciiEscapeOutOfRange = 423,
    EmptyInterpolation = 424,
    InterpolationTooDeep = 425,
    InvalidDigitSeparator = 426,
    MalformedNumericLiteral = 427,
    IntegerLiteralTooLarge = 428,
    EmptyCharLiteral = 429,
    CharLiteralTooLong = 430,
    UnterminatedCharLiteral = 431,
    NonAsciiIdentifier = 432,
    InvalidWhitespace = 433,
    LoneCarriageReturn = 434,
    InvalidUtf8 = 435,
    BackslashOutsideImportPath = 436,
    // E0437 hoi truoc la "keyword viet hoa", kieu `IF` thi bao rieng mot cau.
    // Bo di tu luc chot Pump phan biet hoa thuong (D-4), gio `IF` chi la mot
    // cai ten binh thuong thoi. De trong so 437, dung dung lai cho cai khac,
    // may file .err cu ngoai kia con ghi no.
    ReservedWord = 438,
    OperatorNotInPump = 439,
    UnclosedBracketAtEof = 440,
    UnmatchedClosingBracket = 441,

    // ---- parser, E0450 - E0499 ----
    UnexpectedToken = 450,
    ExpectedToken = 451,
    ImportAfterDeclaration = 452,
    DuplicateImportBinding = 453,
    StructLiteralInHeader = 454,
    ChainedComparison = 455,
    ChainedRange = 456,
    AssignmentInExpression = 457,
    InvalidAssignmentTarget = 458,
    StatementHasNoEffect = 459,
    RequiredParameterAfterDefault = 460,
    VariadicNotLast = 461,
    VariadicWithDefault = 462,
    MultipleVariadicParameters = 463,
    PositionalAfterNamed = 464,
    InterfaceMethodHasBody = 465,
    EmptyVariantPayload = 466,
    EnumWithoutVariants = 467,
    OneTupleNotAllowed = 468,
    EmptyTupleNotAllowed = 469,
    NestedFunctionDeclaration = 470,
    BareErrorReturnType = 471,
    TopLevelLet = 472,
    CommaBetweenStructMembers = 473,
    FloatLiteralPattern = 474,
    InterpolationInPattern = 475,
    DuplicateVisibility = 476,
    VisibilityOnImplements = 477,
    MapKeyBeginsWithBrace = 478,
    UnclosedDelimiter = 479,
    ExpectedStatement = 480,
    ExpectedExpression = 481,
    ExpectedType = 482,
    ExpectedPattern = 483,
    RefutablePatternInBinding = 484,
    MultilineImportPath = 485,

    // ---- resolver, E0500 - E0549 ----
    UnknownIdentifier = 500,
    UnknownType = 501,
    UnknownModule = 502,
    UnknownField = 503,
    UnknownMethod = 504,
    UnknownVariant = 50,
    DuplicateDeclaration = 506,
    ShadowsPredeclaredName = 507,
    UnusedImport = 508,
    PrivateAccess = 509,
    ThisOutsideMethod = 255,
    ModuleNotFound = 511,
    CircularImport = 512,
    CircularConstInitialisation = 513,
    DuplicateField = 514,
    DuplicateMethod = 515,
    DuplicateVariant = 516,
    DuplicateParameter = 517,
    DuplicatePatternBinding = 518,
    MissingMain = 519,
    InvalidMainSignature = 520,
    BreakOutsideLoop = 521,
    ContinueOutsideLoop = 522,
    SelfReferentialClosure = 523,
    DuplicateGenericParameter = 524,
    UnknownInterface = 525,
    UnusedLocal = 526,
    VariantVisibilityMismatch = 528,

    // ---- type checker, E0550 - E0649 ----
    TypeMismatch = 550,
    NotCallable = 551,
    WrongArgumentCount = 552,
    UnknownNamedArgument = 553,
    ArgumentSuppliedTwice = 554,
    MissingArgument = 555,
    VariadicPassedByName = 556,
    NamedArgumentThroughValue = 557,
    NoImplicitConversion = 558,
    ConditionNotBool = 559,
    NotIterable = 560,
    NotIndexable = 561,
    StringNotIndexable = 562,
    CannotAssignToConst = 563,
    CannotAssignToLoopBinding = 564,
    CannotAssignToThis = 565,
    NestedOptional = 566,
    NestedErrorType = 567,
    ErrorTypeOutsideReturn = 568,
    UnhandledError = 569,
    PropagateErrorInNonFailable = 570,
    PropagateNullInNonOptional = 571,
    FailOutsideFailable = 572,
    CatchOnNonFailable = 573,
    CatchAfterPropagate = 574,
    CatchBlockFallsThrough = 575,
    NonExhaustiveMatch = 576,
    UnreachableMatchArm = 577,
    OrPatternBindingMismatch = 578,
    MissingReturn = 579,
    ReturnValueInVoidFunction = 580,
    MissingReturnValue = 581,
    LiteralOutOfRange = 582,
    ConstantOverflow = 583,
    DivisionByZeroConstant = 584,
    NotAConstantExpression = 585,
    InterfaceNotSatisfied = 586,
    MethodOnUnboundedGeneric = 587,
    CannotInferType = 588,
    FloatNotHashable = 589,
    WrongTypeArgumentCount = 590,
    MissingStructField = 591,
    DuplicateStructFieldInit = 592,
    UnknownStructField = 593,
    NotAStruct = 594,
    NotAnEnum = 595,
    InvalidConversion = 596,
    InvalidInterpolation = 597,
    ComparisonNotSupported = 598,
    ArithmeticOnNonNumeric = 599,
    BitwiseOnNonInteger = 600,
    LogicalOnNonBool = 601,
    NegateUnsigned = 602,
    RangeEndpointNotInt = 603,
    InvalidRangePattern = 604,
    PatternTypeMismatch = 605,
    TupleIndexOutOfRange = 606,
    NotATuple = 607,
    CharArithmetic = 608,
    TurbofishNotAllowed = 609,
    InterfaceMethodDefaultParameter = 610,
    ImplementsGenericSubject = 611,
    NoTruthiness = 612,
    MutableCaptureOfLoopBinding = 613,

    // ---- lowering and code generation, E0650 - E0699 ----
    MonomorphisationDepthExceeded = 650,
    UnsupportedConstruct = 651,
    CodegenFailed = 652,
    ObjectEmissionFailed = 653,
    UnsupportedTarget = 654,

    // ---- driver, linker and I/O, E0700 - E0749 ----
    CannotReadFile = 700,
    CannotWriteFile = 701,
    LinkerNotFound = 702,
    LinkFailed = 703,
    InvalidCommandLine = 704,
    EntryFileNotFound = 705,
    RuntimeLibraryNotFound = 706,
    CompilationStopped = 707,
}

impl ErrorCode {
    pub fn number(self) -> u16 {
        self as u16
    }

    /// Mot dong tieu de mac dinh, dung khi cho goi khong co gi ro hon.
    pub fn title(self) -> &'static str {
        use ErrorCode::*;
        match self {
            UnexpectedCharacter => "unexpected character",
            UnterminatedBlockComment => "unterminated block comment",
            UnterminatedString => "unterminated string literal",
            NewlineInString => "a string literal may not contain a raw newline",
            UnknownEscape => "unknown escape sequence",
            InvalidUnicodeEscape => "invalid Unicode escape",
            AsciiEscapeOutOfRange => "an ASCII escape may not exceed 0x7F",
            EmptyInterpolation => "empty interpolation",
            InterpolationTooDeep => "interpolation nested too deeply",
            InvalidDigitSeparator => "a digit separator must sit between two digits",
            MalformedNumericLiteral => "malformed numeric literal",
            IntegerLiteralTooLarge => "integer literal is too large",
            EmptyCharLiteral => "empty character literal",
            CharLiteralTooLong => "a character literal holds exactly one Unicode scalar value",
            UnterminatedCharLiteral => "unterminated character literal",
            NonAsciiIdentifier => "identifiers are ASCII only",
            InvalidWhitespace => "invalid whitespace character",
            LoneCarriageReturn => "a carriage return must be followed by a line feed",
            InvalidUtf8 => "source is not valid UTF-8",
            BackslashOutsideImportPath => "unexpected backslash",
            ReservedWord => "reserved for a future version of Pump",
            OperatorNotInPump => "this operator does not exist in Pump 1.0",
            UnclosedBracketAtEof => "unclosed bracket at end of file",
            UnmatchedClosingBracket => "unmatched closing bracket",

            UnexpectedToken => "unexpected token",
            ExpectedToken => "expected a different token here",
            ImportAfterDeclaration => "imports must appear at the top of the file",
            DuplicateImportBinding => "two imports bind the same name",
            StructLiteralInHeader => {
                "struct literals are not allowed in the header of if / while / for / match"
            }
            ChainedComparison => "comparison operators cannot be chained",
            ChainedRange => "range operators cannot be chained",
            AssignmentInExpression => "assignment is a statement, not an expression",
            InvalidAssignmentTarget => "invalid assignment target",
            StatementHasNoEffect => "this expression has no effect",
            RequiredParameterAfterDefault => "a required parameter may not follow a defaulted one",
            VariadicNotLast => "a variadic parameter must be last",
            VariadicWithDefault => "a variadic parameter may not have a default",
            MultipleVariadicParameters => "a function may have at most one variadic parameter",
            PositionalAfterNamed => "positional arguments must come before named arguments",
            InterfaceMethodHasBody => "interface methods have no body in Pump 1.0",
            EmptyVariantPayload => "a variant with an empty payload list is written without ()",
            EnumWithoutVariants => "an enum must declare at least one variant",
            OneTupleNotAllowed => "Pump has no 1-tuples",
            EmptyTupleNotAllowed => "Pump has no empty tuple",
            NestedFunctionDeclaration => "nested named functions are not permitted in Pump 1.0",
            BareErrorReturnType => "a bare `!` is not a return type",
            TopLevelLet => "module-level state must be declared with `const`",
            CommaBetweenStructMembers => "struct members are separated by a newline or `;`",
            FloatLiteralPattern => "floating-point values cannot be matched",
            InterpolationInPattern => "a string pattern may not contain an interpolation",
            DuplicateVisibility => "duplicate visibility modifier",
            VisibilityOnImplements => "an `implements` declaration takes no visibility",
            MapKeyBeginsWithBrace => "a map key expression may not begin with `{`",
            UnclosedDelimiter => "unclosed delimiter",
            ExpectedStatement => "expected a statement",
            ExpectedExpression => "expected an expression",
            ExpectedType => "expected a type",
            ExpectedPattern => "expected a pattern",
            RefutablePatternInBinding => "this pattern may fail to match",
            MultilineImportPath => "an import path must lie on one line",

            UnknownIdentifier => "cannot find this name in scope",
            UnknownType => "cannot find this type in scope",
            UnknownModule => "cannot find this module",
            UnknownField => "no such field",
            UnknownMethod => "no such method",
            UnknownVariant => "no such variant",
            DuplicateDeclaration => "this name is already declared in this scope",
            ShadowsPredeclaredName => "this predeclared name cannot be shadowed",
            UnusedImport => "unused import",
            PrivateAccess => "this item is private to its module",
            ThisOutsideMethod => "`this` is only bound inside a method",
            ModuleNotFound => "no source file for this module",
            CircularImport => "circular import",
            CircularConstInitialisation => "circular initialisation of module constants",
            DuplicateField => "duplicate field",
            DuplicateMethod => "duplicate method",
            DuplicateVariant => "duplicate variant",
            DuplicateParameter => "duplicate parameter name",
            DuplicatePatternBinding => "a pattern may bind a name at most once",
            MissingMain => "the root module must declare `fn main()`",
            InvalidMainSignature => "invalid signature for `main`",
            BreakOutsideLoop => "`break` outside a loop",
            ContinueOutsideLoop => "`continue` outside a loop",
            SelfReferentialClosure => "this binding is not yet initialised",
            DuplicateGenericParameter => "duplicate generic parameter",
            UnknownInterface => "cannot find this interface",
            UnusedLocal => "unused local binding",
            VariantVisibilityMismatch => "a variant's visibility must match its enum",

            TypeMismatch => "type mismatch",
            NotCallable => "this value is not callable",
            WrongArgumentCount => "wrong number of arguments",
            UnknownNamedArgument => "no parameter with this name",
            ArgumentSuppliedTwice => "argument supplied twice",
            MissingArgument => "missing argument",
            VariadicPassedByName => "a variadic parameter cannot be passed by name",
            NamedArgumentThroughValue => "named arguments need a statically known declaration",
            NoImplicitConversion => "Pump has no implicit numeric conversions",
            ConditionNotBool => "a condition must have type `bool`",
            NotIterable => "this value cannot be iterated",
            NotIndexable => "this value cannot be indexed",
            StringNotIndexable => "`string` cannot be indexed with `[]`",
            CannotAssignToConst => "cannot assign to a `const` binding",
            CannotAssignToLoopBinding => "cannot assign to a `for` binding",
            CannotAssignToThis => "cannot assign to `this`",
            NestedOptional => "optionals do not nest",
            NestedErrorType => "error types do not nest",
            ErrorTypeOutsideReturn => "`!` may only be used on a function's return type",
            UnhandledError => "unhandled error",
            PropagateErrorInNonFailable => {
                "`!` requires an enclosing function with a failable return type"
            }
            PropagateNullInNonOptional => {
                "`?` requires an enclosing function with an optional return type"
            }
            FailOutsideFailable => {
                "`fail` requires an enclosing function with a failable return type"
            }
            CatchOnNonFailable => "`catch` applied to an expression that cannot fail",
            CatchAfterPropagate => "`!` has already consumed the failure",
            CatchBlockFallsThrough => "a `catch` block must not fall through",
            NonExhaustiveMatch => "non-exhaustive match",
            UnreachableMatchArm => "unreachable match arm",
            OrPatternBindingMismatch => {
                "every alternative of an or-pattern must bind the same names"
            }
            MissingReturn => "not every path returns a value",
            ReturnValueInVoidFunction => "this function returns nothing",
            MissingReturnValue => "this function must return a value",
            LiteralOutOfRange => "literal does not fit its type",
            ConstantOverflow => "overflow in a constant expression",
            DivisionByZeroConstant => "division by a zero constant",
            NotAConstantExpression => "not a constant expression",
            InterfaceNotSatisfied => "this type does not satisfy the interface",
            MethodOnUnboundedGeneric => "cannot call a method on an unbounded type parameter",
            CannotInferType => "cannot infer this type",
            FloatNotHashable => "`float` may not be a map key or a set element",
            WrongTypeArgumentCount => "wrong number of type arguments",
            MissingStructField => "missing field in struct literal",
            DuplicateStructFieldInit => "field initialised twice",
            UnknownStructField => "no such field on this struct",
            NotAStruct => "this type is not a struct",
            NotAnEnum => "this type is not an enum",
            InvalidConversion => "invalid conversion",
            InvalidInterpolation => "this type cannot be interpolated into a string",
            ComparisonNotSupported => "these values cannot be compared",
            ArithmeticOnNonNumeric => "arithmetic requires a numeric type",
            BitwiseOnNonInteger => "bitwise operators require `int` or `uint`",
            LogicalOnNonBool => "logical operators require `bool`",
            NegateUnsigned => "`uint` cannot be negated",
            RangeEndpointNotInt => "range endpoints must be `int`",
            InvalidRangePattern => "invalid range pattern",
            PatternTypeMismatch => "this pattern does not match the scrutinee type",
            TupleIndexOutOfRange => "tuple index out of range",
            NotATuple => "this type is not a tuple",
            CharArithmetic => "arithmetic on `char` is not allowed",
            TurbofishNotAllowed => "explicit type arguments are not allowed here",
            InterfaceMethodDefaultParameter => "an interface method may not give a default value",
            ImplementsGenericSubject => {
                "the subject of `implements` must be a non-generic named type"
            }
            NoTruthiness => "Pump has no truthiness",
            MutableCaptureOfLoopBinding => "cannot assign to a captured `for` binding",

            MonomorphisationDepthExceeded => "generic instantiation is too deep",
            UnsupportedConstruct => "this construct is not supported by the backend yet",
            CodegenFailed => "code generation failed",
            ObjectEmissionFailed => "could not write the object file",
            UnsupportedTarget => "unsupported target",

            CannotReadFile => "cannot read file",
            CannotWriteFile => "cannot write file",
            LinkerNotFound => "cannot find rust-lld",
            LinkFailed => "linking failed",
            InvalidCommandLine => "invalid command line",
            EntryFileNotFound => "entry file not found",
            RuntimeLibraryNotFound => "cannot find the Pump runtime library",
            CompilationStopped => "compilation stopped after earlier errors",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "E{:04}", self.number())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_start_at_e0417() {
        assert_eq!(ErrorCode::UnexpectedCharacter.to_string(), "E0417");
    }

    #[test]
    fn renders_a_caret_under_the_span() {
        let mut sources = SourceMap::new();
        let file = sources.add("app/main.pump", "fn main() {\n    let x = @1\n}\n");
        let span = Span::new(file, 24, 25, 2, 13);
        let error = CompileError::at(
            ErrorCode::UnexpectedCharacter,
            span,
            "unexpected character `@`",
        )
        .with_caret("no attribute sigil in Pump 1.0")
        .with_help("Pump 1.0 has no attributes");

        let rendered = error.render(&sources);
        assert!(rendered.contains("error[E0417]: unexpected character `@`"));
        assert!(rendered.contains("--> app/main.pump:2:13"));
        assert!(rendered.contains("2 |     let x = @1"));
        assert!(rendered.contains("^ no attribute sigil in Pump 1.0"));
        assert!(rendered.contains("= help: Pump 1.0 has no attributes"));
    }

    #[test]
    fn line_lookup_matches_the_source_map() {
        let mut sources = SourceMap::new();
        let id = sources.add("t.pump", "a\nbb\nccc\n");
        let file = sources.get(id).unwrap();
        assert_eq!(file.line_of(0), 1);
        assert_eq!(file.line_of(3), 2);
        assert_eq!(file.line_text(3), "ccc");
        assert_eq!(file.line_count(), 4);
    }
}
