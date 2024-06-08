// de quy xuong. Token vao, mot SourceUnit ra.
//
// ca cai kho cua file nay nam o muc 9 cua spec:
//
//  * 9.1 - o trong header
//  struct hay ...
// cai nay  * 9.2
//  * 9.3, 9.4 - `!` voi `?` phan biet nhau chi bang vi tri thoi.
//  * 9.5 - `name:` trong danh sach doi so la doi so co ten, nhin truoc 2
//    token la ra.
//  * 9.6 - chi so tuple thi lexer da tach san thanh mot kind rieng roi.
//
// Gap loi thi day vao roi di tiep, mot lan chay bao duoc nhieu loi. Vong lap
// nao khong chac dung thi deu so cursor truoc va sau, khong tien duoc token
// nao thi ep tien mot cai. Khong lam the la no quay tit. Dung hoi sao t biet.
//
// TODO(furimeo): viet lai theo precedence table cho gon

use crate::ast::*;
use crate::errors::{CompileError, Diagnostics, ErrorCode};
use crate::token::{FileId, Span, Token, TokenKind, TokenValue};

/// Parse one file into a SourceUnit.
pub fn parse(
    file: FileId,
    module_path: Vec<String>,
    tokens: &[Token],
    ids: &mut NodeIdAllocator,
    diagnostics: &mut Diagnostics,
) -> SourceUnit {
    Parser::new(file, tokens, ids, diagnostics).parse_source_unit(module_path)
}

// 9.1: cho nao tat composite thi bao loi phai ve dung cai literal ma nguoi ta
// vua go. Truoc ...
// viet map o ...
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum NoComposite {
    Header,
    Catch,
}

impl NoComposite {
    fn struct_help(self) -> &'static str {
        match self {
            NoComposite::Header => "wrap it in parentheses: `if user == (User { name: \"x\" }) {`",
            NoComposite::Catch => "wrap it in parentheses: `catch (Config { ... })`",
        }
    }

    fn map_help(self) -> &'static str {
        match self {
            NoComposite::Header => "wrap it in parentheses: `if table == ({ \"a\": 1 }) {`",
            NoComposite::Catch => "wrap it in parentheses: `catch ({ \"a\": 1 })`",
        }
    }
}

// ===== bang do uu tien (grammar/precedence.md) =====
//
// t go tay bang nay tu file precedence.md, thu tu la thu tu cua bang do,
// dung doi

const LEVEL_MULTIPLICATIVE: u8 = 3;
const LEVEL_ADDITIVE: u8 = 4;
const LEVEL_SHIFT: u8 = 5;
const LEVEL_BIT_AND: u8 = 6;
const LEVEL_BIT_XOR: u8 = 7;
const LEVEL_BIT_OR: u8 = 8;
const LEVEL_COMPARISON: u8 = 9;
const LEVEL_LOGICAL_AND: u8 = 10;
const LEVEL_LOGICAL_OR: u8 = 11;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Associativity {
    Left,
    None,
}

#[derive(Clone, Copy, Debug)]
struct BinaryEntry {
    op: BinaryOp,
    level: u8,
    associativity: Associativity,
}

fn binary_entry(kind: TokenKind) -> Option<BinaryEntry> {
    use Associativity::{Left, None as NonAssoc};
    use TokenKind as T;

    let (op, level, associativity) = match kind {
        T::Star => (BinaryOp::Mul, LEVEL_MULTIPLICATIVE, Left),
        T::Slash => (BinaryOp::Div, LEVEL_MULTIPLICATIVE, Left),
        T::Percent => (BinaryOp::Rem, LEVEL_MULTIPLICATIVE, Left),
        T::Plus => (BinaryOp::Add, LEVEL_ADDITIVE, Left),
        T::Minus => (BinaryOp::Sub, LEVEL_ADDITIVE, Left),
        T::Shl => (BinaryOp::Shl, LEVEL_SHIFT, Left),
        T::Shr => (BinaryOp::Shr, LEVEL_SHIFT, Left),
        T::Amp => (BinaryOp::BitAnd, LEVEL_BIT_AND, Left),
        T::Caret => (BinaryOp::BitXor, LEVEL_BIT_XOR, Left),
        T::Pipe => (BinaryOp::BitOr, LEVEL_BIT_OR, Left),
        T::EqEq => (BinaryOp::Eq, LEVEL_COMPARISON, NonAssoc),
        T::BangEq => (BinaryOp::Ne, LEVEL_COMPARISON, NonAssoc),
        T::Lt => (BinaryOp::Lt, LEVEL_COMPARISON, NonAssoc),
        T::Gt => (BinaryOp::Gt, LEVEL_COMPARISON, NonAssoc),
        T::LtEq => (BinaryOp::Le, LEVEL_COMPARISON, NonAssoc),
        T::GtEq => (BinaryOp::Ge, LEVEL_COMPARISON, NonAssoc),
        T::AmpAmp => (BinaryOp::And, LEVEL_LOGICAL_AND, Left),
        T::PipePipe => (BinaryOp::Or, LEVEL_LOGICAL_OR, Left),
        _ => return None,
    };
    Some(BinaryEntry {
        op,
        level,
        associativity,
    })
}

fn assignment2(kind: TokenKind) -> Option<AssignOp> {
    let op = match kind {
        TokenKind::Eq => AssignOp::Assign,
        TokenKind::PlusEq => AssignOp::Add,
        TokenKind::MinusEq => AssignOp::Sub,
        TokenKind::StarEq => AssignOp::Mul,
        TokenKind::SlashEq => AssignOp::Div,
        TokenKind::PercentEq => AssignOp::Rem,
        _ => return None,
    };
    Some(op)
}

fn range_is_inclusive(kind: TokenKind) -> Option<bool> {
    match kind {
        TokenKind::DotDot => Some(false),
        TokenKind::DotDotEq => Some(true),
        _ => None,
    }
}

fn do_expression(kind: TokenKind) -> bool {
    use TokenKind::*;
    matches!(
        kind,
        IntLit
            | FloatLit
            | CharLit
            | StringStart
            | True
            | False
            | Null
            | This
            | Ident
            | LBracket
            | LBrace
            | LParen
            | Set
            | Fn
            | Bang
            | Minus
    )
}

fn is_ter(kind: TokenKind) -> bool {
    matches!(kind, TokenKind::Terminator | TokenKind::Semicolon)
}

fn begins_statement(kind: TokenKind) -> bool {
    use TokenKind::*;
    matches!(
        kind,
        Let | Const
            | If
            | While
            | For
            | Match
            | Return
            | Fail
            | Break
            | Continue
            | Fn
            | Struct
            | Enum
            | Interface
            | Implements
            | Import
            | Pub
            | Private
    )
}

// ===== moc gia tri ra khoi token =====

fn ident_text(token: &Token) -> String {
    token.ident().unwrap_or_default().to_string()
}

fn int_val(token: &Token) -> u64 {
    match token.value {
        TokenValue::Int(value) => value,
        _ => 0,
    }
}

fn float_value(token: &Token) -> f64 {
    match token.value {
        TokenValue::Float(value) => value,
        _ => 0.0,
    }
}

fn char_value(token: &Token) -> char {
    match token.value {
        TokenValue::Char(value) => value,
        _ => '\0',
    }
}

fn string_text(token: &Token) -> String {
    match &token.value {
        TokenValue::Str(value) => value.clone(),
        _ => String::new(),
    }
}

// ===== ban than

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Cursor {
    index: usize,
    split: bool,
}

struct Parser<'a> {
    tokens: &'a [Token],
    index: usize,
    split: Option<Token>,
    eof: Token,
    last_span: Span,
    ids: &'a mut NodeIdAllocator,
    diagnostics: &'a mut Diagnostics,
    no_composite: bool,
    composite_help: NoComposite,
    match_arm: bool,
    last_report: Option<(u32, ErrorCode)>,
}

impl<'a> Parser<'a> {
    fn new(
        file: FileId,
        tokens: &'a [Token],
        ids: &'a mut NodeIdAllocator,
        diagnostics: &'a mut Diagnostics,
    ) -> Parser<'a> {
        let start = Span::new(file, 0, 0, 1, 1);
        let eof = tokens
            .last()
            .filter(|token| token.kind == TokenKind::Eof)
            .cloned()
            .unwrap_or_else(|| Token::new(TokenKind::Eof, start));
        Parser {
            tokens,
            index: 0,
            split: None,
            eof,
            last_span: start,
            ids,
            diagnostics,
            no_composite: false,
            composite_help: NoComposite::Header,
            match_arm: false,
            last_report: None,
        }
    }

    // ---- doc token ----

    fn cursor(&self) -> Cursor {
        Cursor {
            index: self.index,
            split: self.split.is_some(),
        }
    }

    fn token_at(&self, index: usize) -> &Token {
        self.tokens.get(index).unwrap_or(&self.eof)
    }

    fn peek_nth(&self, offset: usize) -> &Token {
        match (&self.split, offset) {
            (Some(token), 0) => token,
            (Some(_), _) => self.token_at(self.index + offset - 1),
            (None, _) => self.token_at(self.index + offset),
        }
    }

    fn peek(&self) -> &Token {
        self.peek_nth(0)
    }

    fn kind(&self) -> TokenKind {
        self.peek().kind
    }

    fn kind_nth(&self, offset: usize) -> TokenKind {
        self.peek_nth(offset).kind
    }

    fn span(&self) -> Span {
        self.peek().span
    }

    fn at(&self, kind: TokenKind) -> bool {
        self.kind() == kind
    }

    fn at_end(&self) -> bool {
        self.at(TokenKind::Eof)
    }

    fn advance(&mut self) -> Token {
        let token = match self.split.take() {
            Some(token) => token,
            None => {
                let token = self.token_at(self.index).clone();
                if token.kind != TokenKind::Eof {
                    self.index += 1;
                }
                token
            }
        };
        self.last_span = token.span;
        token
    }

    fn eat(&mut self, kind: TokenKind) -> Option<Token> {
        if self.at(kind) {
            Some(self.advance())
        } else {
            None
        }
    }

    fn expect(&mut self, kind: TokenKind) -> Option<Token> {
        if self.at(kind) {
            return Some(self.advance());
        }
        let span = self.span();
        let found = self.kind();
        self.report(
            CompileError::at(
                ErrorCode::ExpectedToken,
                span,
                format!("expected {}, found {}", kind.describe(), found.describe()),
            )
            .with_caret(format!("expected {}", kind.describe())),
        );
        None
    }

    fn expect_ident(&mut self) -> Option<Ident> {
        let token = self.expect(TokenKind::Ident)?;
        Some(Ident::new(ident_text(&token), token.span))
    }

    // ---- dung node ----

    fn expr(&mut self, kind: ExprKind, span: Span) -> Expr {
        Expr {
            id: self.ids.allocate(),
            kind,
            span,
        }
    }

    fn type_expr(&mut self, kind: TypeExprKind, span: Span) -> TypeExpr {
        TypeExpr {
            id: self.ids.allocate(),
            kind,
            span,
        }
    }

    fn pattern(&mut self, kind: PatternKind, span: Span) -> Pattern {
        Pattern {
            id: self.ids.allocate(),
            kind,
            span,
        }
    }

    fn stmt(&mut self, kind: StmtKind, span: Span) -> Stmt {
        Stmt {
            id: self.ids.allocate(),
            kind,
            span,
        }
    }

    fn expr_placeholder(&mut self, span: Span) -> Expr {
        self.expr(ExprKind::Null, span)
    }

    fn type_placeholder(&mut self, span: Span) -> TypeExpr {
        let path = TypePath {
            module: None,
            name: Ident::new("void", span),
            span,
        };
        self.type_expr(
            TypeExprKind::Path {
                path,
                args: Vec::new(),
            },
            span,
        )
    }

    // ---- bao loi ----

    fn report(&mut self, error: CompileError) {
        let key = (error.primary.span.start, error.code);
        if self.last_report == Some(key) {
            return;
        }
        self.last_report = Some(key);
        self.diagnostics.push(error);
    }

    fn report_reserved(&mut self, span: Span) -> bool {
        let kind = self.kind();
        if !kind.is_reserved() {
            return false;
        }
        let spelling = match kind {
            TokenKind::ReservedWord => format!("`{}`", ident_text(self.peek())),
            other => other.describe().to_string(),
        };
        let mut e = CompileError::at(
            ErrorCode::ReservedWord,
            span,
            format!("{spelling} is reserved for a future version of Pump"),
        )
        .with_caret("reserved")
        .with_help("choose another name, or drop it");
        if kind == TokenKind::Defer {
            // ast::DeferStmt nam san trong ast.rs
            // van chua dung den. Xem TODO.txt.
            e = e.with_note("chua lam");
        }
        self.report(e);
        true
    }

    // ---- che do

    fn without_composites<T>(&mut self, help: NoComposite, f: impl FnOnce(&mut Self) -> T) -> T {
        let saved = (self.no_composite, self.composite_help);
        self.no_composite = true;
        self.composite_help = help;
        let result = f(self);
        (self.no_composite, self.composite_help) = saved;
        result
    }

    fn with_composites<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let saved = std::mem::replace(&mut self.no_composite, false);
        let result = f(self);
        self.no_composite = saved;
        result
    }

    fn inside_block<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let saved = (self.no_composite, self.match_arm);
        self.no_composite = false;
        self.match_arm = false;
        let result = f(self);
        (self.no_composite, self.match_arm) = saved;
        result
    }

    fn inside_match_arm<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let saved = std::mem::replace(&mut self.match_arm, true);
        let result = f(self);
        self.match_arm = saved;
        result
    }

    // ---- terminator

    fn eat_terminators(&mut self) -> bool {
        let mut ate = false;
        while is_ter(self.kind()) {
            self.advance();
            ate = true;
        }
        ate
    }

    fn eat_composite_separators(&mut self) -> bool {
        let mut ate = false;
        while self.at(TokenKind::Comma) || is_ter(self.kind()) {
            self.advance();
            ate = true;
        }
        ate
    }

    fn expect_terminator(&mut self) {
        if self.eat_terminators() {
            return;
        }
        if matches!(self.kind(), TokenKind::RBrace | TokenKind::Eof) {
            return;
        }
        // 13.4.1 ke ra `,`, terminator va `}`
        // mot dong trong spec thi khong co cai nao, nen o trong nhanh thieu
        // cai nay ca ba cung
        // se khong toi duoc `=>` cua no.
        if self.match_arm {
            return;
        }
        let span = self.span();
        let found = self.kind();
        self.report(
            CompileError::at(
                ErrorCode::ExpectedToken,
                span,
                format!("expected a newline or `;` here, found {}", found.describe()),
            )
            .with_caret("expected the end of this statement"),
        );
        self.recover_to_statement_boundary();
    }

    fn skip_to_closing_brace(&mut self) {
        let mut depth = 0usize;
        loop {
            match self.kind() {
                TokenKind::Eof => return,
                TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => {
                    depth += 1;
                    self.advance();
                }
                TokenKind::RBrace if depth == 0 => return,
                TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                    depth -= 1;
                    self.advance();
                }
                _ => {
                    self.advance();
                }
            }
        }
    }

    fn recover_to_statement_boundary(&mut self) {
        let mut depth = 0usize;
        loop {
            match self.kind() {
                TokenKind::Eof => return,
                TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => {
                    depth += 1;
                    self.advance();
                }
                TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                    self.advance();
                }
                kind if depth == 0 && is_ter(kind) => {
                    self.advance();
                    return;
                }
                kind if depth == 0 && begins_statement(kind) => return,
                _ => {
                    self.advance();
                }
            }
        }
    }

    // cai nay ---- tach dau

    fn at_angle_close(&self) -> bool {
        matches!(
            self.kind(),
            TokenKind::Gt | TokenKind::Shr | TokenKind::GtEq
        )
    }

    fn expect_angle_close(&mut self) -> Span {
        let token = self.peek().clone();
        match token.kind {
            TokenKind::Gt => {
                self.advance();
                token.span
            }
            TokenKind::Shr => self.split_leading_gt(&token, TokenKind::Gt),
            TokenKind::GtEq => self.split_leading_gt(&token, TokenKind::Eq),
            _ => {
                self.expect(TokenKind::Gt);
                self.last_span
            }
        }
    }

    fn split_leading_gt(&mut self, token: &Token, remainder: TokenKind) -> Span {
        self.advance();
        let span = token.span;
        let head = Span::new(
            span.file,
            span.start,
            span.start + 1,
            span.line,
            span.column,
        );
        let tail = Span::new(
            span.file,
            span.start + 1,
            span.end,
            span.line,
            span.column + 1,
        );
        self.split = Some(Token::new(remainder, tail));
        self.last_span = head;
        head
    }
}

// ===== cau truc

impl Parser<'_> {
    fn parse_source_unit(mut self, module_path: Vec<String>) -> SourceUnit {
        let id = self.ids.allocate();
        let start = self.span();
        let mut imports: Vec<Import> = Vec::new();
        let mut declarations = Vec::new();
        let mut bound: Vec<(String, Span)> = Vec::new();
        let mut seen_declaration = false;

        self.eat_terminators();
        while !self.at_end() {
            let before = self.cursor();
            if self.at(TokenKind::Import) {
                if let Some(import) = self.parse_import() {
                    if seen_declaration {
                        self.report(
                            CompileError::new(ErrorCode::ImportAfterDeclaration, import.span)
                                .with_help("move every `import` above the first declaration"),
                        );
                    }
                    self.check_import_binding(&import, &mut bound);
                    imports.push(import);
                }
            } else if let Some(declaration) = self.parse_top_level_declaration() {
                seen_declaration = true;
                declarations.push(declaration);
            }
            self.eat_terminators();
            if self.cursor() == before {
                self.advance();
            }
        }

        SourceUnit {
            id,
            module_path,
            imports,
            declarations,
            span: start.to(self.last_span),
        }
    }

    fn check_import_binding(&mut self, import: &Import, bound: &mut Vec<(String, Span)>) {
        let name = import.bound_name().clone();
        let previous = bound
            .iter()
            .find(|(existing, _)| *existing == name.name)
            .map(|(_, span)| *span);
        match previous {
            Some(previous) => self.report(
                CompileError::at(
                    ErrorCode::DuplicateImportBinding,
                    name.span,
                    format!("`{}` is already bound by another import", name.name),
                )
                .with_secondary(previous, "first bound here")
                .with_help("rename one of them with `as`"),
            ),
            None => bound.push((name.name, name.span)),
        }
    }

    fn parse_import(&mut self) -> Option<Import> {
        let id = self.ids.allocate();
        let start = self.expect(TokenKind::Import)?.span;

        let mut path = vec![self.expect_ident()?];
        while self.at(TokenKind::Backslash) {
            let backslash = self.advance();
            if is_ter(self.kind()) || self.at(TokenKind::Eof) {
                self.report(
                    CompileError::new(ErrorCode::MultilineImportPath, backslash.span)
                        .with_caret("the path continues after this backslash"),
                );
                break;
            }
            match self.expect_ident() {
                Some(segment) => path.push(segment),
                None => break,
            }
        }

        let alias = if self.eat(TokenKind::As).is_some() {
            self.expect_ident()
        } else {
            None
        };
        self.expect_terminator();

        Some(Import {
            id,
            path,
            alias,
            span: start.to(self.last_span),
        })
    }

    fn parse_top_level_declaration(&mut self) -> Option<Declaration> {
        let visibility = self.parse_visibility();
        match self.kind() {
            TokenKind::Fn => self
                .parse_function_declaration(visibility)
                .map(Declaration::Function),
            TokenKind::Struct => self.parse_struct(visibility).map(Declaration::Struct),
            TokenKind::Enum => self.parse_enum(visibility).map(Declaration::Enum),
            TokenKind::Interface => self.parse_interface(visibility).map(Declaration::Interface),
            TokenKind::Const => self.parse_const(visibility).map(Declaration::Const),
            TokenKind::Implements => {
                if let Some(span) = visibility.span {
                    self.report(CompileError::new(ErrorCode::VisibilityOnImplements, span));
                }
                self.parse_implements().map(Declaration::Implements)
            }
            // 10.3.3: bien o muc module bat buoc `const`. Cu ghi nhan no la
            // const de phan con lai cua file con co nghia ma doc.
            TokenKind::Let => {
                let declaration = self.parse_let()?;
                self.report(
                    CompileError::new(ErrorCode::TopLevelLet, declaration.span)
                        .with_help("write `const` instead"),
                );
                Some(Declaration::Const(ConstDecl {
                    id: declaration.id,
                    visibility,
                    pattern: declaration.pattern,
                    ty: declaration.ty,
                    value: declaration.value,
                    span: declaration.span,
                }))
            }
            found => {
                let span = self.span();
                if !self.report_reserved(span) {
                    self.report(
                        CompileError::at(
                            ErrorCode::UnexpectedToken,
                            span,
                            format!(
                                "expected a top-level declaration, found {}",
                                found.describe()
                            ),
                        )
                        .with_help(
                            "a file holds `import`, `fn`, `struct`, `enum`, `interface`, \
                             `const` and `implements`",
                        ),
                    );
                }
                self.recover_to_statement_boundary();
                None
            }
        }
    }

    fn parse_visibility(&mut self) -> Visibility {
        let mut visibility = Visibility::implicit_private();
        if let Some(token) = self.eat(TokenKind::Pub) {
            visibility = Visibility {
                kind: VisibilityKind::Public,
                span: Some(token.span),
            };
        } else if let Some(token) = self.eat(TokenKind::Private) {
            visibility = Visibility {
                kind: VisibilityKind::Private,
                span: Some(token.span),
            };
        }
        while matches!(self.kind(), TokenKind::Pub | TokenKind::Private) {
            let extra = self.advance();
            self.report(CompileError::new(
                ErrorCode::DuplicateVisibility,
                extra.span,
            ));
        }
        visibility
    }

    fn parse_implements(&mut self) -> Option<ImplementsDecl> {
        let id = self.ids.allocate();
        let start = self.expect(TokenKind::Implements)?.span;
        let subject = self.expect_ident()?;
        self.expect(TokenKind::Colon);

        let mut interfaces = Vec::new();
        loop {
            let before = self.cursor();
            match self.parse_type_path() {
                Some(path) => interfaces.push(path),
                None => break,
            }
            if self.eat(TokenKind::Comma).is_none() {
                break;
            }
            if is_ter(self.kind()) || self.at(TokenKind::Eof) {
                break;
            }
            if self.cursor() == before {
                self.advance();
            }
        }
        self.expect_terminator();

        Some(ImplementsDecl {
            id,
            subject,
            interfaces,
            span: start.to(self.last_span),
        })
    }
}

// ===== khai bao (12) =====

impl Parser<'_> {
    fn parse_function_declaration(&mut self, visibility: Visibility) -> Option<FunctionDecl> {
        let id = self.ids.allocate();
        let start = self.expect(TokenKind::Fn)?.span;
        let name = self.expect_ident()?;
        let generics = self.parse_generic_parameters();
        let params = self.parse_parameters();
        let return_type = self.parse_optional_return_type();
        let body = self.parse_block();

        Some(FunctionDecl {
            id,
            visibility,
            name,
            generics,
            params,
            return_type,
            body,
            span: start.to(self.last_span),
        })
    }

    fn parse_optional_return_type(&mut self) -> Option<TypeExpr> {
        if self.eat(TokenKind::Colon).is_none() {
            return None;
        }
        Some(self.parse_type())
    }

    // -- generics. Doan nay t va vao he 2025, sau ca file nay gan mot nam,
    // -- nen style no khong giong may
    // cai nay -- thi so
    fn parse_generic_parameters(&mut self) -> Vec<GenericParam> {
        let mut out: Vec<GenericParam> = Vec::new();
        match self.eat(TokenKind::Lt) {
            None => return out,
            Some(_) => {}
        }
        loop {
            if self.at_angle_close() || self.at_end() {
                break;
            }
            let mark = self.cursor();
            let id = self.ids.allocate();
            let name = match self.expect_ident() {
                Some(n) => n,
                None => break,
            };
            let mut bounds = Vec::new();
            if self.eat(TokenKind::Colon).is_some() {
                bounds = self.parse_type_bounds();
            }
            let start = name.span;
            out.push(GenericParam {
                id,
                name: name.clone(),
                bounds,
                span: start.to(self.last_span),
            });
            if self.eat(TokenKind::Comma).is_none() {
                break;
            }
            if self.cursor() == mark {
                self.advance();
            }
        }
        self.expect_angle_close();
        return out;
    }

    // rang buoc: `T: Printable +
    fn parse_type_bounds(&mut self) -> Vec<TypePath> {
        let mut out: Vec<TypePath> = Vec::new();
        loop {
            let mark = self.cursor();
            match self.parse_type_path() {
                Some(path) => out.push(path),
                None => break,
            }
            if self.eat(TokenKind::Plus).is_none() {
                break;
            }
            if self.cursor() == mark {
                self.advance();
            }
        }
        return out;
    }

    fn parse_parameters(&mut self) -> Vec<Param> {
        if self.expect(TokenKind::LParen).is_none() {
            return Vec::new();
        }
        let params = self.with_composites(|parser| parser.parse_parameter_list());
        self.expect(TokenKind::RParen);
        self.validate_parameter_order(&params);
        params
    }

    fn parse_parameter_list(&mut self) -> Vec<Param> {
        let mut params = Vec::new();
        while !self.at(TokenKind::RParen) && !self.at_end() {
            let before = self.cursor();
            match self.parse_parameter() {
                Some(param) => params.push(param),
                None => break,
            }
            if self.eat(TokenKind::Comma).is_none() {
                break;
            }
            if self.cursor() == before {
                self.advance();
            }
        }
        params
    }

    fn parse_parameter(&mut self) -> Option<Param> {
        let id = self.ids.allocate();
        let name = self.expect_ident()?;
        let start = name.span;
        self.expect(TokenKind::Colon);

        let variadic = self.eat(TokenKind::Ellipsis).is_some();
        let ty = self.parse_type();
        let default = if self.eat(TokenKind::Eq).is_some() {
            Some(self.parse_expression())
        } else {
            None
        };

        let kind = match (variadic, default) {
            (true, Some(default)) => {
                let span = default.span;
                self.report(
                    CompileError::new(ErrorCode::VariadicWithDefault, span)
                        .with_help("a variadic parameter already defaults to no arguments"),
                );
                ParamKind::Variadic
            }
            (true, None) => ParamKind::Variadic,
            (false, Some(default)) => ParamKind::Default(default),
            (false, None) => ParamKind::Required,
        };

        Some(Param {
            id,
            name,
            ty,
            kind,
            span: start.to(self.last_span),
        })
    }

    fn validate_parameter_order(&mut self, params: &[Param]) {
        let mut defaulted: Option<Span> = None;
        let mut variadic: Option<Span> = None;
        for param in params {
            if let Some(first) = variadic {
                if matches!(param.kind, ParamKind::Variadic) {
                    self.report(
                        CompileError::new(ErrorCode::MultipleVariadicParameters, param.span)
                            .with_secondary(first, "the first variadic parameter is here"),
                    );
                } else {
                    self.report(
                        CompileError::new(ErrorCode::VariadicNotLast, first)
                            .with_secondary(param.span, "this parameter follows it"),
                    );
                }
                continue;
            }
            match param.kind {
                ParamKind::Required => {
                    if let Some(first) = defaulted {
                        self.report(
                            CompileError::new(ErrorCode::RequiredParameterAfterDefault, param.span)
                                .with_secondary(first, "this parameter has a default")
                                .with_help("give this one a default too, or move it earlier"),
                        );
                    }
                }
                ParamKind::Default(_) => defaulted = Some(param.span),
                ParamKind::Variadic => variadic = Some(param.span),
            }
        }
    }

    fn parse_struct(&mut self, visibility: Visibility) -> Option<StructDecl> {
        let id = self.ids.allocate();
        let start = self.expect(TokenKind::Struct)?.span;
        let name = self.expect_ident()?;
        let generics = self.parse_generic_parameters();
        self.expect(TokenKind::LBrace);

        let mut members = Vec::new();
        self.eat_terminators();
        while !self.at(TokenKind::RBrace) && !self.at_end() {
            let before = self.cursor();
            self.reject_comma_between_members();
            if self.at(TokenKind::RBrace) {
                break;
            }
            let member_visibility = self.parse_visibility();
            if self.at(TokenKind::Fn) {
                if let Some(method) = self.parse_function_declaration(member_visibility) {
                    members.push(StructMember::Method(method));
                }
            } else if let Some(field) = self.parse_field(member_visibility) {
                members.push(StructMember::Field(field));
            } else {
                self.recover_to_statement_boundary();
            }
            self.eat_terminators();
            if self.cursor() == before {
                self.advance();
            }
        }
        self.expect(TokenKind::RBrace);

        Some(StructDecl {
            id,
            visibility,
            name,
            generics,
            members,
            span: start.to(self.last_span),
        })
    }

    fn parse_field(&mut self, visibility: Visibility) -> Option<FieldDecl> {
        let id = self.ids.allocate();
        let name = self.expect_ident()?;
        let start = name.span;
        self.expect(TokenKind::Colon);
        let ty = self.parse_type();
        self.expect_member_terminator();
        Some(FieldDecl {
            id,
            visibility,
            name,
            ty,
            span: start.to(self.last_span),
        })
    }

    fn expect_member_terminator(&mut self) {
        if self.at(TokenKind::Comma) {
            self.reject_comma_between_members();
            return;
        }
        self.expect_terminator();
    }

    fn reject_comma_between_members(&mut self) {
        while self.at(TokenKind::Comma) {
            let comma = self.advance();
            self.report(
                CompileError::new(ErrorCode::CommaBetweenStructMembers, comma.span)
                    .with_help("remove the comma"),
            );
            self.eat_terminators();
        }
    }

    fn parse_enum(&mut self, visibility: Visibility) -> Option<EnumDecl> {
        let id = self.ids.allocate();
        let start = self.expect(TokenKind::Enum)?.span;
        let name = self.expect_ident()?;
        let generics = self.parse_generic_parameters();
        self.expect(TokenKind::LBrace);

        let mut members = Vec::new();
        self.eat_terminators();
        while !self.at(TokenKind::RBrace) && !self.at_end() {
            let before = self.cursor();
            self.reject_comma_between_members();
            if self.at(TokenKind::RBrace) {
                break;
            }
            let member_visibility = self.parse_visibility();
            if self.at(TokenKind::Fn) {
                if let Some(method) = self.parse_function_declaration(member_visibility) {
                    members.push(EnumMember::Method(method));
                }
            } else if let Some(variant) = self.parse_variant(member_visibility) {
                members.push(EnumMember::Variant(variant));
            } else {
                self.recover_to_statement_boundary();
            }
            self.eat_terminators();
            if self.cursor() == before {
                self.advance();
            }
        }
        self.expect(TokenKind::RBrace);

        let span = start.to(self.last_span);
        let declaration = EnumDecl {
            id,
            visibility,
            name,
            generics,
            members,
            span,
        };
        if declaration.variants().next().is_none() {
            self.report(
                CompileError::new(ErrorCode::EnumWithoutVariants, span)
                    .with_caret("this enum declares no variants"),
            );
        }
        Some(declaration)
    }

    fn parse_variant(&mut self, visibility: Visibility) -> Option<VariantDecl> {
        let id = self.ids.allocate();
        let name = self.expect_ident()?;
        let start = name.span;

        let mut payload = Vec::new();
        if self.at(TokenKind::LParen) {
            let open = self.advance().span;
            payload = self.with_composites(|parser| parser.parse_type_list(TokenKind::RParen));
            let close = self
                .expect(TokenKind::RParen)
                .map(|token| token.span)
                .unwrap_or(self.last_span);
            if payload.is_empty() {
                self.report(
                    CompileError::new(ErrorCode::EmptyVariantPayload, open.to(close))
                        .with_help(format!("write `{}` on its own", name.name)),
                );
            }
        }
        self.expect_member_terminator();

        Some(VariantDecl {
            id,
            visibility,
            name,
            payload,
            span: start.to(self.last_span),
        })
    }

    fn parse_interface(&mut self, visibility: Visibility) -> Option<InterfaceDecl> {
        let id = self.ids.allocate();
        let start = self.expect(TokenKind::Interface)?.span;
        let name = self.expect_ident()?;
        let generics = self.parse_generic_parameters();
        self.expect(TokenKind::LBrace);

        let mut methods = Vec::new();
        self.eat_terminators();
        while !self.at(TokenKind::RBrace) && !self.at_end() {
            let before = self.cursor();
            match self.parse_interface_method() {
                Some(method) => methods.push(method),
                None => self.recover_to_statement_boundary(),
            }
            self.eat_terminators();
            if self.cursor() == before {
                self.advance();
            }
        }
        self.expect(TokenKind::RBrace);

        Some(InterfaceDecl {
            id,
            visibility,
            name,
            generics,
            methods,
            span: start.to(self.last_span),
        })
    }

    fn parse_interface_method(&mut self) -> Option<InterfaceMethod> {
        let id = self.ids.allocate();
        let start = self.expect(TokenKind::Fn)?.span;
        let name = self.expect_ident()?;
        let generics = self.parse_generic_parameters();
        let params = self.parse_parameters();
        let return_type = self.parse_optional_return_type();

        if self.at(TokenKind::LBrace) {
            let body = self.parse_block();
            self.report(
                CompileError::new(ErrorCode::InterfaceMethodHasBody, body.span)
                    .with_help("declare the body on the implementing type instead"),
            );
        }
        self.expect_terminator();

        Some(InterfaceMethod {
            id,
            name,
            generics,
            params,
            return_type,
            span: start.to(self.last_span),
        })
    }

    fn parse_let(&mut self) -> Option<LetDecl> {
        let id = self.ids.allocate();
        let start = self.expect(TokenKind::Let)?.span;
        let pattern = self.parse_irrefutable_pattern();
        let ty = if self.eat(TokenKind::Colon).is_some() {
            Some(self.parse_type())
        } else {
            None
        };
        self.expect(TokenKind::Eq);
        let value = self.parse_expression();
        self.expect_terminator();
        Some(LetDecl {
            id,
            pattern,
            ty,
            value,
            span: start.to(self.last_span),
        })
    }

    fn parse_const(&mut self, visibility: Visibility) -> Option<ConstDecl> {
        let id = self.ids.allocate();
        let start = self.expect(TokenKind::Const)?.span;
        let pattern = self.parse_irrefutable_pattern();
        let ty = if self.eat(TokenKind::Colon).is_some() {
            Some(self.parse_type())
        } else {
            None
        };
        self.expect(TokenKind::Eq);
        let value = self.parse_expression();
        self.expect_terminator();
        Some(ConstDecl {
            id,
            visibility,
            pattern,
            ty,
            value,
            span: start.to(self.last_span),
        })
    }
}

// ===== kieu (11) =====

impl Parser<'_> {
    fn parse_type(&mut self) -> TypeExpr {
        let mut ty = self.parse_primary_type();
        loop {
            let kind = match self.kind() {
                TokenKind::Question => TypeExprKind::Optional(Box::new(ty)),
                TokenKind::Bang => TypeExprKind::Failable(Box::new(ty)),
                _ => return ty,
            };
            let suffix = self.advance().span;
            let span = match &kind {
                TypeExprKind::Optional(inner) | TypeExprKind::Failable(inner) => {
                    inner.span.to(suffix)
                }
                _ => suffix,
            };
            ty = self.type_expr(kind, span);
        }
    }

    fn parse_primary_type(&mut self) -> TypeExpr {
        let start = self.span();
        match self.kind() {
            TokenKind::Ident => {
                let Some(path) = self.parse_type_path() else {
                    return self.type_placeholder(start);
                };
                let args = if self.at(TokenKind::Lt) {
                    self.parse_type_arguments()
                } else {
                    Vec::new()
                };
                let span = start.to(self.last_span);
                self.type_expr(TypeExprKind::Path { path, args }, span)
            }
            TokenKind::LBracket => self.parse_array_or_map_type(),
            TokenKind::Set => self.parse_set_type(),
            TokenKind::LParen => self.parse_tuple_or_grouped_type(),
            TokenKind::Fn => self.parse_function_type(),
            // 11.7 / E-24: ham co the loi ma khong tra ve gi la `void!`, nen
            // `!` dung mot minh phai co thong bao rieng
            TokenKind::Bang => {
                let bang = self.advance().span;
                self.report(
                    CompileError::new(ErrorCode::BareErrorReturnType, bang)
                        .with_help("write `void!` for a failable function that returns nothing"),
                );
                let inner = self.type_placeholder(bang);
                self.type_expr(TypeExprKind::Failable(Box::new(inner)), bang)
            }
            found => {
                if !self.report_reserved(start) {
                    self.report(
                        CompileError::at(
                            ErrorCode::ExpectedType,
                            start,
                            format!("expected a type, found {}", found.describe()),
                        )
                        .with_caret("expected a type"),
                    );
                }
                self.type_placeholder(start)
            }
        }
    }

    fn parse_type_path(&mut self) -> Option<TypePath> {
        let first = self.expect_ident()?;
        let start = first.span;
        if self.at(TokenKind::Dot) && self.kind_nth(1) == TokenKind::Ident {
            self.advance();
            let name = self.expect_ident()?;
            let span = start.to(name.span);
            return Some(TypePath {
                module: Some(first),
                name,
                span,
            });
        }
        Some(TypePath {
            module: None,
            name: first,
            span: start,
        })
    }

    fn parse_type_arguments(&mut self) -> Vec<TypeExpr> {
        let mut args = Vec::new();
        if self.eat(TokenKind::Lt).is_none() {
            return args;
        }
        while !self.at_angle_close() && !self.at_end() {
            let before = self.cursor();
            args.push(self.parse_type());
            if self.eat(TokenKind::Comma).is_none() {
                break;
            }
            if self.cursor() == before {
                self.advance();
            }
        }
        self.expect_angle_close();
        args
    }

    fn parse_array_or_map_type(&mut self) -> TypeExpr {
        let start = self.advance().span;
        let element = self.with_composites(|parser| parser.parse_type());
        if self.eat(TokenKind::Colon).is_some() {
            let value = self.with_composites(|parser| parser.parse_type());
            self.expect(TokenKind::RBracket);
            let span = start.to(self.last_span);
            return self.type_expr(
                TypeExprKind::Map {
                    key: Box::new(element),
                    value: Box::new(value),
                },
                span,
            );
        }
        self.expect(TokenKind::RBracket);
        let span = start.to(self.last_span);
        self.type_expr(TypeExprKind::Array(Box::new(element)), span)
    }

    fn parse_set_type(&mut self) -> TypeExpr {
        let start = self.advance().span;
        self.expect(TokenKind::Lt);
        let element = self.parse_type();
        self.expect_angle_close();
        let span = start.to(self.last_span);
        self.type_expr(TypeExprKind::Set(Box::new(element)), span)
    }

    fn parse_tuple_or_grouped_type(&mut self) -> TypeExpr {
        let start = self.advance().span;
        if let Some(close) = self.eat(TokenKind::RParen) {
            let span = start.to(close.span);
            self.report(
                CompileError::new(ErrorCode::EmptyTupleNotAllowed, span)
                    .with_help("the absence of a value is spelled `void`"),
            );
            return self.type_placeholder(span);
        }

        let mut elements = vec![self.with_composites(|parser| parser.parse_type())];
        let mut trailing_comma = false;
        while self.eat(TokenKind::Comma).is_some() {
            if self.at(TokenKind::RParen) {
                trailing_comma = true;
                break;
            }
            let before = self.cursor();
            elements.push(self.with_composites(|parser| parser.parse_type()));
            if self.cursor() == before {
                self.advance();
            }
        }
        self.expect(TokenKind::RParen);
        let span = start.to(self.last_span);

        match elements.len() {
            1 if trailing_comma => {
                self.report(
                    CompileError::new(ErrorCode::OneTupleNotAllowed, span)
                        .with_help("drop the trailing comma to write a grouped type"),
                );
                let inner = elements.pop().expect("one element");
                self.type_expr(TypeExprKind::Group(Box::new(inner)), span)
            }
            1 => {
                let inner = elements.pop().expect("one element");
                self.type_expr(TypeExprKind::Group(Box::new(inner)), span)
            }
            _ => self.type_expr(TypeExprKind::Tuple(elements), span),
        }
    }

    fn parse_function_type(&mut self) -> TypeExpr {
        let start = self.advance().span;
        self.expect(TokenKind::LParen);

        let mut params = Vec::new();
        let mut variadic = None;
        self.with_composites(|parser| {
            while !parser.at(TokenKind::RParen) && !parser.at_end() {
                let before = parser.cursor();
                if parser.eat(TokenKind::Ellipsis).is_some() {
                    variadic = Some(Box::new(parser.parse_type()));
                    parser.eat(TokenKind::Comma);
                    break;
                }
                params.push(parser.parse_type());
                if parser.eat(TokenKind::Comma).is_none() {
                    break;
                }
                if parser.cursor() == before {
                    parser.advance();
                }
            }
        });
        self.expect(TokenKind::RParen);

        let return_type = if self.eat(TokenKind::Colon).is_some() {
            Some(Box::new(self.parse_type()))
        } else {
            None
        };
        let span = start.to(self.last_span);

        self.type_expr(
            TypeExprKind::Function(FunctionTypeExpr {
                params,
                variadic,
                return_type,
                span,
            }),
            span,
        )
    }

    fn parse_type_list(&mut self, close: TokenKind) -> Vec<TypeExpr> {
        let mut types = Vec::new();
        while !self.at(close) && !self.at_end() {
            let before = self.cursor();
            types.push(self.parse_type());
            if self.eat(TokenKind::Comma).is_none() {
                break;
            }
            if self.cursor() == before {
                self.advance();
            }
        }
        types
    }
}

// ===== statement (13) =====

impl Parser<'_> {
    fn parse_block(&mut self) -> Block {
        let id = self.ids.allocate();
        let Some(open) = self.expect(TokenKind::LBrace) else {
            let span = self.span();
            return Block {
                id,
                statements: Vec::new(),
                span,
            };
        };
        let statements = self.inside_block(|parser| parser.parse_statement_list());
        let close = self
            .expect(TokenKind::RBrace)
            .map(|token| token.span)
            .unwrap_or(self.last_span);
        Block {
            id,
            statements,
            span: open.span.to(close),
        }
    }

    fn parse_statement_list(&mut self) -> Vec<Stmt> {
        let mut statements = Vec::new();
        self.eat_terminators();
        while !self.at(TokenKind::RBrace) && !self.at_end() {
            let before = self.cursor();
            if let Some(statement) = self.parse_statement() {
                statements.push(statement);
            }
            self.eat_terminators();
            if self.cursor() == before {
                self.advance();
            }
        }
        statements
    }

    fn parse_statement(&mut self) -> Option<Stmt> {
        let start = self.span();
        let kind = match self.kind() {
            TokenKind::Let => StmtKind::Let(self.parse_let()?),
            TokenKind::Const => StmtKind::Const(self.parse_const(Visibility::implicit_private())?),
            TokenKind::If => StmtKind::If(self.parse_if()?),
            TokenKind::While => StmtKind::While(self.parse_while()?),
            TokenKind::For => StmtKind::For(self.parse_for()?),
            TokenKind::Match => StmtKind::Match(self.parse_match()?),
            TokenKind::Return => self.parse_return()?,
            TokenKind::Fail => self.parse_fail()?,
            TokenKind::Break => {
                self.advance();
                self.expect_terminator();
                StmtKind::Break
            }
            TokenKind::Continue => {
                self.advance();
                self.expect_terminator();
                StmtKind::Continue
            }
            // 13.0.1: cau lenh bat dau bang `{` luon la block, khong bao gio
            // la map literal
            TokenKind::LBrace => StmtKind::Block(self.parse_block()),
            // 12.1.5 / E-41: `fn` trong block la closure, nen mot cai co ten
            // la loi rieng
            TokenKind::Fn if self.kind_nth(1) == TokenKind::Ident => {
                let declaration = self.parse_function_declaration(Visibility::implicit_private())?;
                self.report(
                    CompileError::new(ErrorCode::NestedFunctionDeclaration, declaration.span)
                        .with_help(format!(
                            "write `let {} = fn(...) {{ ... }}` instead",
                            declaration.name.name
                        )),
                );
                return None;
            }
            TokenKind::Import => {
                let span = self.span();
                self.report(CompileError::new(ErrorCode::ImportAfterDeclaration, span));
                self.parse_import();
                return None;
            }
            TokenKind::Struct
            | TokenKind::Enum
            | TokenKind::Interface
            | TokenKind::Implements
            | TokenKind::Pub
            | TokenKind::Private => {
                let found = self.kind();
                self.report(
                    CompileError::at(
                        ErrorCode::UnexpectedToken,
                        start,
                        format!("{} is only allowed at the top level", found.describe()),
                    )
                    .with_caret("not allowed inside a block"),
                );
                self.advance();
                self.recover_to_statement_boundary();
                return None;
            }
            _ => return self.parse_expression_statement(),
        };
        let span = start.to(self.last_span);
        Some(self.stmt(kind, span))
    }

    fn parse_return(&mut self) -> Option<StmtKind> {
        self.expect(TokenKind::Return)?;
        let value = if self.at_statement_end() {
            None
        } else {
            Some(self.parse_expression())
        };
        self.expect_terminator();
        Some(StmtKind::Return(value))
    }

    fn parse_fail(&mut self) -> Option<StmtKind> {
        self.expect(TokenKind::Fail)?;
        let value = self.parse_expression();
        self.expect_terminator();
        Some(StmtKind::Fail(value))
    }

    fn at_statement_end(&self) -> bool {
        is_ter(self.kind())
            || matches!(self.kind(), TokenKind::RBrace | TokenKind::Eof)
            || (self.match_arm && self.at(TokenKind::Comma))
    }

    fn parse_expression_statement(&mut self) -> Option<Stmt> {
        let start = self.span();
        if !do_expression(self.kind()) {
            let found = self.kind();
            if !self.report_reserved(start) {
                self.report(
                    CompileError::at(
                        ErrorCode::ExpectedStatement,
                        start,
                        format!("expected a statement, found {}", found.describe()),
                    )
                    .with_caret("expected a statement"),
                );
            }
            self.recover_to_statement_boundary();
            return None;
        }

        let target = self.parse_catch_expression();
        if let Some(op) = assignment2(self.kind()) {
            self.advance();
            if !target.is_lvalue() {
                self.report(
                    CompileError::new(ErrorCode::InvalidAssignmentTarget, target.span).with_help(
                        "only a name, `this.field`, a field or an index can be assigned to",
                    ),
                );
            }
            let value = self.parse_expression();
            self.expect_terminator();
            let span = start.to(self.last_span);
            let statement = AssignStmt {
                target,
                op,
                value,
                span,
            };
            return Some(self.stmt(StmtKind::Assign(statement), span));
        }

        // 13.2.1: cau lenh bieu thuc ma khong co loi goi nao thi la code
        // chet
        if !target.contains_call() {
            self.report(
                CompileError::new(ErrorCode::StatementHasNoEffect, target.span)
                    .with_help("only an expression containing a call may stand as a statement"),
            );
        }
        self.expect_terminator();
        let span = start.to(self.last_span);
        Some(self.stmt(StmtKind::Expr(target), span))
    }

    fn parse_if(&mut self) -> Option<IfStmt> {
        let start = self.expect(TokenKind::If)?.span;
        let condition =
            self.without_composites(NoComposite::Header, |parser| parser.parse_expression());
        let then_block = self.parse_block();
        let else_branch = if self.eat(TokenKind::Else).is_some() {
            if self.at(TokenKind::If) {
                Some(ElseBranch::If(Box::new(self.parse_if()?)))
            } else {
                Some(ElseBranch::Block(self.parse_block()))
            }
        } else {
            None
        };
        Some(IfStmt {
            condition,
            then_block,
            else_branch,
            span: start.to(self.last_span),
        })
    }

    fn parse_while(&mut self) -> Option<WhileStmt> {
        let start = self.expect(TokenKind::While)?.span;
        let condition =
            self.without_composites(NoComposite::Header, |parser| parser.parse_expression());
        let body = self.parse_block();
        Some(WhileStmt {
            condition,
            body,
            span: start.to(self.last_span),
        })
    }

    fn parse_for(&mut self) -> Option<ForStmt> {
        let start = self.expect(TokenKind::For)?.span;
        let pattern = self.parse_irrefutable_pattern();
        self.expect(TokenKind::In);
        let iterable =
            self.without_composites(NoComposite::Header, |parser| parser.parse_expression());
        let body = self.parse_block();
        Some(ForStmt {
            pattern,
            iterable,
            body,
            span: start.to(self.last_span),
        })
    }

    fn parse_match(&mut self) -> Option<MatchStmt> {
        let start = self.expect(TokenKind::Match)?.span;
        let scrutinee =
            self.without_composites(NoComposite::Header, |parser| parser.parse_expression());
        self.expect(TokenKind::LBrace);

        let mut arms = Vec::new();
        self.eat_terminators();
        while !self.at(TokenKind::RBrace) && !self.at_end() {
            let before = self.cursor();
            match self.parse_match_arm() {
                Some(arm) => arms.push(arm),
                None => self.recover_to_statement_boundary(),
            }
            // 13.4.1: sau mot nhanh thi nhan `,`, terminator, hoac `}`
            self.eat(TokenKind::Comma);
            self.eat_terminators();
            if self.cursor() == before {
                self.advance();
            }
        }
        self.expect(TokenKind::RBrace);

        Some(MatchStmt {
            scrutinee,
            arms,
            span: start.to(self.last_span),
        })
    }

    fn parse_match_arm(&mut self) -> Option<MatchArm> {
        let id = self.ids.allocate();
        let start = self.span();
        let pattern = self.parse_pattern();
        let guard = if self.eat(TokenKind::If).is_some() {
            Some(self.without_composites(NoComposite::Header, |parser| parser.parse_expression()))
        } else {
            None
        };
        self.expect(TokenKind::FatArrow)?;

        let body = if self.at(TokenKind::LBrace) {
            MatchArmBody::Block(self.parse_block())
        } else {
            let statement = self.inside_match_arm(|parser| parser.parse_statement())?;
            MatchArmBody::Stmt(Box::new(statement))
        };

        Some(MatchArm {
            id,
            pattern,
            guard,
            body,
            span: start.to(self.last_span),
        })
    }
}

// ===== bieu thuc (14, precedence.md) =====

impl Parser<'_> {
    fn parse_expression(&mut self) -> Expr {
        let expr = self.parse_catch_expression();
        if assignment2(self.kind()).is_some() {
            let operator = self.advance();
            self.report( CompileError::new(ErrorCode::AssignmentInExpression, operator.span) .with_help("assign on its own line, then use the binding here"), );
            self.parse_catch_expression();
        }
        expr
    }

    fn parse_catch_expression(&mut self) -> Expr {
        let mut expr = self.parse_range_expression();
        while self.at(TokenKind::Catch) {
            self.advance();
            let handler = self.parse_catch_tail();
            let span = expr.span.to(handler.span());
            expr = self.expr(
                ExprKind::Catch {
                    operand: Box::new(expr),
                    handler,
                },
                span,
            );
        }
        expr
    }

    fn parse_catch_tail(&mut self) -> CatchHandler {
        if self.at(TokenKind::LBrace) {
            return CatchHandler::Discard(self.parse_block());
        }
        let binding_form = self.kind() == TokenKind::Ident
            && self.kind_nth(1) == TokenKind::LBrace
            && !self.looks_like_struct_body(1);
        if binding_form {
            let token = self.advance();
            let name = Ident::new(ident_text(&token), token.span);
            let block = self.parse_block();
            return CatchHandler::Bind { name, block };
        }
        // che do `ns` ...
        // cai loi ma spec doi, thay vi parse sai am tham (14.2)
        let value =
            self.without_composites(NoComposite::Catch, |parser| parser.parse_range_expression());
        CatchHandler::Value(Box::new(value))
    }

    fn parse_range_expression(&mut self) -> Expr {
        let start = self.parse_binary(LEVEL_LOGICAL_OR);
        let Some(inclusive) = range_is_inclusive(self.kind()) else {
            return start;
        };
        self.advance();
        let end = self.parse_binary(LEVEL_LOGICAL_OR);
        let span = start.span.to(end.span);
        let mut range = self.expr(
            ExprKind::Range {
                start: Box::new(start),
                end: Box::new(end),
                inclusive,
            },
            span,
        );

        if range_is_inclusive(self.kind()).is_some() {
            let span = self.span();
            self.report(
                CompileError::new(ErrorCode::ChainedRange, span)
                    .with_help("a range has exactly two endpoints; use parentheses"),
            );
            while let Some(inclusive) = range_is_inclusive(self.kind()) {
                self.advance();
                let next = self.parse_binary(LEVEL_LOGICAL_OR);
                let span = range.span.to(next.span);
                range = self.expr(
                    ExprKind::Range {
                        start: Box::new(range),
                        end: Box::new(next),
                        inclusive,
                    },
                    span,
                );
            }
        }
        range
    }

    fn parse_binary(&mut self, max_level: u8) -> Expr {
        let mut lhs = self.parse_unary();
        let mut previous_non_associative: Option<u8> = None;
        let mut chain_reported = false;

        loop {
            let Some(entry) = binary_entry(self.kind()) else {
                break;
            };
            if entry.level > max_level {
                break;
            }
            if previous_non_associative == Some(entry.level) && !chain_reported {
                chain_reported = true;
                let span = self.span();
                self.report( CompileError::new(ErrorCode::ChainedComparison, span) .with_help("use parentheses, or split with `&&`"), );
            }
            self.advance();
            let rhs = self.parse_binary(entry.level - 1);
            let span = lhs.span.to(rhs.span);
            lhs = self.expr( ExprKind::Binary { op: entry.op, lhs: Box::new(lhs), rhs: Box::new(rhs), }, span, );
            previous_non_associative = match entry.associativity {
                Associativity::None => Some(entry.level),
                Associativity::Left => None,
            };
        }
        lhs
    }

    fn parse_unary(&mut self) -> Expr {
        let op = match self.kind() {
            TokenKind::Bang => UnaryOp::Not,
            TokenKind::Minus => UnaryOp::Neg,
            _ => return self.parse_postfix(),
        };
        let start = self.advance().span;
        let operand = self.parse_unary();
        let span = start.to(operand.span);
        self.expr( ExprKind::Unary { op, operand: Box::new(operand), }, span, )
    }

    fn parse_postfix(&mut self) -> Expr {
        let mut expr = self.parse_primary();
        loop {
            match self.kind() {
                TokenKind::Dot => match self.parse_member_access(expr) {
                    Ok(next) => expr = next,
                    Err(stop) => return stop,
                },
                TokenKind::LParen => {
                    let args = self.parse_call_arguments();
                    let span = expr.span.to(self.last_span);
                    expr = self.expr(
                        ExprKind::Call {
                            callee: Box::new(expr),
                            args,
                        },
                        span,
                    );
                }
                TokenKind::LBracket => {
                    self.advance();
                    let index = self.with_composites(|parser| parser.parse_expression());
                    self.expect(TokenKind::RBracket);
                    let span = expr.span.to(self.last_span);
                    expr = self.expr(
                        ExprKind::Index {
                            base: Box::new(expr),
                            index: Box::new(index),
                        },
                        span,
                    );
                }
                TokenKind::Question => {
                    let span = expr.span.to(self.advance().span);
                    expr = self.expr(ExprKind::NullPropagate(Box::new(expr)), span);
                }
                TokenKind::Bang => {
                    let span = expr.span.to(self.advance().span);
                    expr = self.expr(ExprKind::ErrorPropagate(Box::new(expr)), span);
                }
                TokenKind::ColonColon => {
                    self.advance();
                    if self.expect(TokenKind::Lt).is_none() {
                        return expr;
                    }
                    let args = self.parse_turbofish_arguments();
                    let span = expr.span.to(self.last_span);
                    expr = self.expr(
                        ExprKind::TypeArgs {
                            base: Box::new(expr),
                            args,
                        },
                        span,
                    );
                }
                TokenKind::LBrace => match self.parse_struct_literal_suffix(expr) {
                    Ok(next) => expr = next,
                    Err(stop) => return stop,
                },
                _ => return expr,
            }
        }
    }

    fn parse_member_access(&mut self, base: Expr) -> Result<Expr, Expr> {
        self.advance();
        match self.kind() {
            TokenKind::Ident => {
                let token = self.advance();
                let name = Ident::new(ident_text(&token), token.span);
                let span = base.span.to(token.span);
                Ok(self.expr(
                    ExprKind::Field {
                        base: Box::new(base),
                        name,
                    },
                    span,
                ))
            }
            // cai nay 9.6: chu so
            TokenKind::TupleIndex => {
                let token = self.advance();
                let index = int_val(&token).min(u32::MAX as u64) as u32;
                let span = base.span.to(token.span);
                Ok(self.expr(
                    ExprKind::TupleField {
                        base: Box::new(base),
                        index,
                        index_span: token.span,
                    },
                    span,
                ))
            }
            _ => {
                self.expect(TokenKind::Ident);
                Err(base)
            }
        }
    }

    fn parse_struct_literal_suffix(&mut self, base: Expr) -> Result<Expr, Expr> {
        if !base.is_struct_literal_path() {
            return Err(base);
        }
        if self.no_composite {
            // trong che do `ns` thi `{`
            // cai nay header. chi than
            // struct literal, va do dung la cai loi 9.1 bat phai bao.
            if !self.looks_like_struct_body(0) {
                return Err(base);
            }
            let span = self.span();
            self.report(
                CompileError::new(ErrorCode::StructLiteralInHeader, span)
                    .with_caret("this `{` opens a struct literal")
                    .with_help(self.composite_help.struct_help()),
            );
        }
        let Some((path, type_args)) = struct_literal_path(&base) else {
            return Err(base);
        };
        let start = base.span;
        let fields = self.parse_struct_literal_body();
        let span = start.to(self.last_span);
        let literal = StructLit {
            path,
            type_args,
            fields,
            span,
        };
        Ok(self.expr(ExprKind::StructLit(literal), span))
    }

    fn looks_like_struct_body(&self, brace_offset: usize) -> bool {
        if self.kind_nth(brace_offset) != TokenKind::LBrace {
            return false;
        }
        let mut offset = brace_offset + 1;
        while is_ter(self.kind_nth(offset)) {
            offset += 1;
        }
        self.kind_nth(offset) == TokenKind::Ident && self.kind_nth(offset + 1) == TokenKind::Colon
    }

    fn parse_struct_literal_body(&mut self) -> Vec<FieldInit> {
        if self.expect(TokenKind::LBrace).is_none() {
            return Vec::new();
        }
        let fields = self.with_composites(|parser| {
            let mut fields = Vec::new();
            parser.eat_composite_separators();
            while !parser.at(TokenKind::RBrace) && !parser.at_end() {
                let before = parser.cursor();
                let Some(name) = parser.expect_ident() else {
                    break;
                };
                parser.expect(TokenKind::Colon);
                let value = parser.parse_expression();
                let span = name.span.to(value.span);
                fields.push(FieldInit { name, value, span });
                if !parser.eat_composite_separators() {
                    break;
                }
                if parser.cursor() == before {
                    parser.advance();
                }
            }
            fields
        });
        self.expect(TokenKind::RBrace);
        fields
    }

    fn parse_call_arguments(&mut self) -> Vec<Argument> {
        if self.expect(TokenKind::LParen).is_none() {
            return Vec::new();
        }
        let args = self.with_composites(|parser| parser.parse_argument_list());
        self.expect(TokenKind::RParen);
        args
    }

    fn parse_argument_list(&mut self) -> Vec<Argument> {
        let mut args = Vec::new();
        let mut first_named: Option<Span> = None;
        while !self.at(TokenKind::RParen) && !self.at_end() {
            let before = self.cursor();
            let start = self.span();
            // 9.5: `identifier ":"` mo dau mot doi so co ten, nhin truoc hai
            // token la du
            let name = if self.kind() == TokenKind::Ident && self.kind_nth(1) == TokenKind::Colon {
                let token = self.advance();
                self.advance();
                Some(Ident::new(ident_text(&token), token.span))
            } else {
                None
            };
            let value = self.parse_expression();
            let span = start.to(value.span);
            match (&name, first_named) {
                (Some(_), None) => first_named = Some(span),
                (None, Some(first)) => self.report(
                    CompileError::new(ErrorCode::PositionalAfterNamed, span)
                        .with_secondary(first, "the first named argument is here"),
                ),
                _ => {}
            }
            args.push(Argument { name, value, span });
            if self.eat(TokenKind::Comma).is_none() {
                break;
            }
            if self.cursor() == before {
                self.advance();
            }
        }
        args
    }

    fn parse_turbofish_arguments(&mut self) -> Vec<TypeExpr> {
        let mut args = Vec::new();
        while !self.at_angle_close() && !self.at_end() {
            let before = self.cursor();
            args.push(self.parse_type());
            if self.eat(TokenKind::Comma).is_none() {
                break;
            }
            if self.cursor() == before {
                self.advance();
            }
        }
        self.expect_angle_close();
        args
    }

    fn parse_primary(&mut self) -> Expr {
        let span = self.span();
        let kind = match self.kind() {
            TokenKind::IntLit => {
                let token = self.advance();
                ExprKind::Int(int_val(&token))
            }
            TokenKind::FloatLit => {
                let token = self.advance();
                ExprKind::Float(float_value(&token))
            }
            TokenKind::CharLit => {
                let token = self.advance();
                ExprKind::Char(char_value(&token))
            }
            TokenKind::StringStart => {
                let literal = self.parse_string_literal();
                let span = literal.span;
                return self.expr(ExprKind::Str(literal), span);
            }
            TokenKind::True => {
                self.advance();
                ExprKind::Bool(true)
            }
            TokenKind::False => {
                self.advance();
                ExprKind::Bool(false)
            }
            TokenKind::Null => {
                self.advance();
                ExprKind::Null
            }
            TokenKind::This => {
                self.advance();
                ExprKind::This
            }
            TokenKind::Ident => {
                let token = self.advance();
                ExprKind::Ident(Ident::new(ident_text(&token), token.span))
            }
            TokenKind::LBracket => return self.parse_array_literal(),
            TokenKind::LBrace => return self.parse_map_literal(),
            TokenKind::Set => return self.parse_set_literal(),
            TokenKind::LParen => return self.parse_tuple_or_grouped(),
            TokenKind::Fn => return self.parse_closure(),
            found => {
                if !self.report_reserved(span) {
                    self.report(
                        CompileError::at(
                            ErrorCode::ExpectedExpression,
                            span,
                            format!("expected an expression, found {}", found.describe()),
                        )
                        .with_caret("expected an expression"),
                    );
                }
                return self.expr_placeholder(span);
            }
        };
        self.expr(kind, span)
    }

    fn parse_string_literal(&mut self) -> StringLit {
        let start = self.span();
        self.expect(TokenKind::StringStart);
        let mut parts = Vec::new();
        loop {
            match self.kind() {
                TokenKind::StringText => {
                    let token = self.advance();
                    parts.push(StringPart::Text {
                        value: string_text(&token),
                        span: token.span,
                    });
                }
                TokenKind::InterpStart => {
                    self.advance();
                    let expr = self.with_composites(|parser| parser.parse_expression());
                    self.expect(TokenKind::InterpEnd);
                    parts.push(StringPart::Interp(Box::new(expr)));
                }
                TokenKind::StringEnd => {
                    self.advance();
                    break;
                }
                _ => {
                    self.expect(TokenKind::StringEnd);
                    break;
                }
            }
        }
        StringLit {
            parts,
            span: start.to(self.last_span),
        }
    }

    fn parse_array_literal(&mut self) -> Expr {
        let start = self.advance().span;
        let elements = self.with_composites(|parser| {
            let mut elements = Vec::new();
            while !parser.at(TokenKind::RBracket) && !parser.at_end() {
                let before = parser.cursor();
                elements.push(parser.parse_expression());
                if parser.eat(TokenKind::Comma).is_none() {
                    break;
                }
                if parser.cursor() == before {
                    parser.advance();
                }
            }
            elements
        });
        self.expect(TokenKind::RBracket);
        let span = start.to(self.last_span);
        self.expr(ExprKind::Array(elements), span)
    }

    fn parse_map_literal(&mut self) -> Expr {
        let start = self.span();
        if self.no_composite {
            self.report(
                CompileError::at(
                    ErrorCode::StructLiteralInHeader,
                    start,
                    "map literals are not allowed in the header of if / while / for / match",
                )
                .with_help(self.composite_help.map_help()),
            );
        }
        self.advance();
        let entries = self.with_composites(|parser| {
            let mut entries = Vec::new();
            parser.eat_composite_separators();
            while !parser.at(TokenKind::RBrace) && !parser.at_end() {
                let before = parser.cursor();
                // 14.25: de quyet dinh entry hay element trong dung mot token
                // thi khoa khong ...
                if parser.at(TokenKind::LBrace) {
                    let span = parser.span();
                    parser.report(
                        CompileError::new(ErrorCode::MapKeyBeginsWithBrace, span)
                            .with_help("parenthesise the key"),
                    );
                }
                let key = parser.parse_expression();
                if !parser.at(TokenKind::Colon) {
                    // `{1, 2}` la map literal ma khong co `:`, gan nhu luon
                    // luon la set viet sai (14.18). Noi mot lan thoi chu dung
                    // bao loi tren tung phan tu.
                    let span = parser.span();
                    parser.report(
                        CompileError::at(
                            ErrorCode::ExpectedToken,
                            span,
                            format!(
                                "expected `:` after a map key, found {}",
                                parser.kind().describe()
                            ),
                        )
                        .with_caret("a map entry is written `key: value`")
                        .with_help("for a set, write `set{...}`"),
                    );
                    parser.skip_to_closing_brace();
                    break;
                }
                parser.advance();
                let value = parser.parse_expression();
                let span = key.span.to(value.span);
                entries.push(MapEntry { key, value, span });
                if !parser.eat_composite_separators() {
                    break;
                }
                if parser.cursor() == before {
                    parser.advance();
                }
            }
            entries
        });
        self.expect(TokenKind::RBrace);
        let span = start.to(self.last_span);
        self.expr(ExprKind::Map(entries), span)
    }

    fn parse_set_literal(&mut self) -> Expr {
        let start = self.advance().span;
        if self.expect(TokenKind::LBrace).is_none() {
            let span = start.to(self.last_span);
            return self.expr(ExprKind::Set(Vec::new()), span);
        }
        let elements = self.with_composites(|parser| {
            let mut elements = Vec::new();
            parser.eat_composite_separators();
            while !parser.at(TokenKind::RBrace) && !parser.at_end() {
                let before = parser.cursor();
                elements.push(parser.parse_expression());
                if !parser.eat_composite_separators() {
                    break;
                }
                if parser.cursor() == before {
                    parser.advance();
                }
            }
            elements
        });
        self.expect(TokenKind::RBrace);
        let span = start.to(self.last_span);
        self.expr(ExprKind::Set(elements), span)
    }

    fn parse_tuple_or_grouped(&mut self) -> Expr {
        let start = self.advance().span;
        if let Some(close) = self.eat(TokenKind::RParen) {
            let span = start.to(close.span);
            self.report(CompileError::new(ErrorCode::EmptyTupleNotAllowed, span));
            return self.expr_placeholder(span);
        }

        let (mut elements, trailing_comma) = self.with_composites(|parser| {
            let mut elements = vec![parser.parse_expression()];
            let mut trailing_comma = false;
            while parser.eat(TokenKind::Comma).is_some() {
                if parser.at(TokenKind::RParen) {
                    trailing_comma = true;
                    break;
                }
                let before = parser.cursor();
                elements.push(parser.parse_expression());
                if parser.cursor() == before {
                    parser.advance();
                }
            }
            (elements, trailing_comma)
        });
        self.expect(TokenKind::RParen);
        let span = start.to(self.last_span);

        if elements.len() == 1 {
            if trailing_comma {
                self.report(
                    CompileError::new(ErrorCode::OneTupleNotAllowed, span)
                        .with_help("drop the trailing comma to write a grouped expression"),
                );
            }
            let inner = elements.pop().expect("one element");
            return self.expr(ExprKind::Group(Box::new(inner)), span);
        }
        self.expr(ExprKind::Tuple(elements), span)
    }

    fn parse_closure(&mut self) -> Expr {
        let id = self.ids.allocate();
        let start = self.advance().span;
        let params = self.parse_parameters();
        let return_type = self.parse_optional_return_type();
        let body = self.parse_block();
        let span = start.to(body.span);
        let closure = ClosureExpr {
            id,
            params,
            return_type,
            body,
            span,
        };
        self.expr(ExprKind::Closure(closure), span)
    }
}

fn struct_literal_path(expr: &Expr) -> Option<(TypePath, Vec<TypeExpr>)> {
    match &expr.kind {
        ExprKind::Ident(name) => Some((
            TypePath {
                module: None,
                name: name.clone(),
                span: expr.span,
            },
            Vec::new(),
        )),
        ExprKind::Field { base, name } => match &base.kind {
            ExprKind::Ident(module) => Some((
                TypePath {
                    module: Some(module.clone()),
                    name: name.clone(),
                    span: expr.span,
                },
                Vec::new(),
            )),
            _ => None,
        },
        ExprKind::TypeArgs { base, args } => {
            let (path, _) = struct_literal_path(base)?;
            Some((path, args.clone()))
        }
        _ => None,
    }
}

// ===== pattern (15) =====

impl Parser<'_> {
    fn parse_pattern(&mut self) -> Pattern {
        let first = self.parse_primary_pattern();
        if !self.at(TokenKind::Pipe) {
            return first;
        }
        let start = first.span;
        let mut alternatives = vec![first];
        while self.eat(TokenKind::Pipe).is_some() {
            let before = self.cursor();
            alternatives.push(self.parse_primary_pattern());
            if self.cursor() == before {
                self.advance();
                break;
            }
        }
        let span = start.to(self.last_span);
        self.pattern(PatternKind::Or(alternatives), span)
    }

    fn parse_primary_pattern(&mut self) -> Pattern {
        let span = self.span();
        match self.kind() {
            TokenKind::Underscore => {
                self.advance();
                self.pattern(PatternKind::Wildcard, span)
            }
            TokenKind::Null => {
                self.advance();
                self.pattern(PatternKind::Null, span)
            }
            TokenKind::True => {
                self.advance();
                self.pattern(PatternKind::Bool(true), span)
            }
            TokenKind::False => {
                self.advance();
                self.pattern(PatternKind::Bool(false), span)
            }
            TokenKind::Minus | TokenKind::IntLit | TokenKind::CharLit => {
                self.parse_literal_or_range_pattern()
            }
            // 15.2: so sanh bang tren so thuc khong phai cai nen chac chan de
            // phan nhanh
            TokenKind::FloatLit => {
                self.advance();
                self.report(
                    CompileError::new(ErrorCode::FloatLiteralPattern, span)
                        .with_help("use a comparison in a guard instead"),
                );
                self.pattern(PatternKind::Wildcard, span)
            }
            TokenKind::StringStart => {
                let literal = self.parse_string_literal();
                let span = literal.span;
                match literal.as_plain() {
                    Some(text) => self.pattern(PatternKind::Str(text), span),
                    None => {
                        self.report(CompileError::new(ErrorCode::InterpolationInPattern, span));
                        self.pattern(PatternKind::Wildcard, span)
                    }
                }
            }
            TokenKind::Ident => self.parse_named_pattern(),
            TokenKind::LParen => self.parse_tuple_pattern(),
            found => {
                self.report( CompileError::at( ErrorCode::ExpectedPattern, span, format!("expected a pattern, found {}", found.describe()), )
                    .with_caret("expected a pattern"),
                );
                self.pattern(PatternKind::Wildcard, span)
            }
        }
    }

    fn parse_literal_or_range_pattern(&mut self) -> Pattern {
        let start = self.span();
        let Some(low) = self.parse_range_endpoint() else {
            return self.pattern(PatternKind::Wildcard, start);
        };
        let Some(inclusive) = range_is_inclusive(self.kind()) else {
            let span = start.to(self.last_span);
            let kind = match low {
                RangeEndpoint::Int {
                    magnitude,
                    negative,
                } => PatternKind::Int {
                    magnitude,
                    negative,
                },
                RangeEndpoint::Char(value) => PatternKind::Char(value),
            };
            return self.pattern(kind, span);
        };
        self.advance();
        let Some(high) = self.parse_range_endpoint() else {
            let span = start.to(self.last_span);
            return self.pattern(PatternKind::Wildcard, span);
        };
        let span = start.to(self.last_span);
        self.pattern(
            PatternKind::Range {
                start: low,
                end: high,
                inclusive,
            },
            span,
        )
    }

    fn parse_range_endpoint(&mut self) -> Option<RangeEndpoint> {
        let negative = self.eat(TokenKind::Minus).is_some();
        match self.kind() {
            TokenKind::IntLit => {
                let token = self.advance();
                Some(RangeEndpoint::Int {
                    magnitude: int_val(&token),
                    negative,
                })
            }
            TokenKind::CharLit if !negative => {
                let token = self.advance();
                Some(RangeEndpoint::Char(char_value(&token)))
            }
            found => {
                let span = self.span();
                self.report(
                    CompileError::at(
                        ErrorCode::ExpectedPattern,
                        span,
                        format!(
                            "expected an integer or character literal, found {}",
                            found.describe()
                        ),
                    )
                    .with_caret("expected a literal"),
                );
                None
            }
        }
    }

    fn parse_named_pattern(&mut self) -> Pattern {
        let token = self.advance();
        let name = Ident::new(ident_text(&token), token.span);
        let start = name.span;

        if self.at(TokenKind::Dot) && self.kind_nth(1) == TokenKind::Ident {
            self.advance();
            let variant_token = self.advance();
            let variant = Ident::new(ident_text(&variant_token), variant_token.span);
            let payload = if self.at(TokenKind::LParen) {
                self.advance();
                let patterns = self.parse_pattern_list();
                self.expect(TokenKind::RParen);
                Some(patterns)
            } else {
                None
            };
            let span = start.to(self.last_span);
            return self.pattern(
                PatternKind::Variant {
                    enum_name: name,
                    variant,
                    payload,
                },
                span,
            );
        }

        // 15.9: cho cua pattern khong phai cho cua bieu thuc, nen che do `ns`
        // khong ap dung va `User { .. }` khong can dau ngoac
        if self.at(TokenKind::LBrace) {
            return self.parse_struct_pattern(name);
        }

        self.pattern(PatternKind::Binding(name), start)
    }

    fn parse_struct_pattern(&mut self, name: Ident) -> Pattern {
        let start = name.span;
        self.advance();
        let mut fields = Vec::new();
        let mut rest = false;

        self.eat_composite_separators();
        while !self.at(TokenKind::RBrace) && !self.at_end() {
            let before = self.cursor();
            if self.eat(TokenKind::DotDot).is_some() {
                rest = true;
                self.eat_composite_separators();
                break;
            }
            let Some(field) = self.expect_ident() else {
                break;
            };
            let field_start = field.span;
            let pattern = if self.eat(TokenKind::Colon).is_some() {
                Some(self.parse_pattern())
            } else {
                None
            };
            fields.push(FieldPattern {
                name: field,
                pattern,
                span: field_start.to(self.last_span),
            });
            if !self.eat_composite_separators() {
                break;
            }
            if self.cursor() == before {
                self.advance();
            }
        }
        self.expect(TokenKind::RBrace);

        let span = start.to(self.last_span);
        self.pattern(PatternKind::Struct { name, fields, rest }, span)
    }

    fn parse_tuple_pattern(&mut self) -> Pattern {
        let start = self.advance().span;
        let mut elements = self.parse_pattern_list();
        self.expect(TokenKind::RParen);
        let span = start.to(self.last_span);

        match elements.len() {
            0 => {
                self.report(CompileError::new(ErrorCode::EmptyTupleNotAllowed, span));
                self.pattern(PatternKind::Wildcard, span)
            }
            1 => {
                self.report(
                    CompileError::new(ErrorCode::OneTupleNotAllowed, span)
                        .with_help("a pattern in parentheses is not a tuple; remove them"),
                );
                let inner = elements.pop().expect("one element");
                Pattern {
                    id: inner.id,
                    kind: inner.kind,
                    span,
                }
            }
            _ => self.pattern(PatternKind::Tuple(elements), span),
        }
    }

    fn parse_pattern_list(&mut self) -> Vec<Pattern> {
        let mut patterns = Vec::new();
        while !self.at(TokenKind::RParen) && !self.at_end() {
            let before = self.cursor();
            patterns.push(self.parse_pattern());
            if self.eat(TokenKind::Comma).is_none() {
                break;
            }
            if self.cursor() == before {
                self.advance();
            }
        }
        patterns
    }

    fn parse_irrefutable_pattern(&mut self) -> IrrefutablePattern {
        let pattern = self.parse_pattern();
        self.into_irrefutable(pattern)
    }

    fn into_irrefutable(&mut self, pattern: Pattern) -> IrrefutablePattern {
        let span = pattern.span;
        let kind = match pattern.kind {
            PatternKind::Wildcard => IrrefutablePatternKind::Wildcard,
            PatternKind::Binding(name) => IrrefutablePatternKind::Binding(name),
            PatternKind::Tuple(elements) => {
                let mut converted = Vec::with_capacity(elements.len());
                for element in elements {
                    converted.push(self.into_irrefutable(element));
                }
                IrrefutablePatternKind::Tuple(converted)
            }
            _ => {
                self.report(
                    CompileError::new(ErrorCode::RefutablePatternInBinding, span).with_help(
                        "`let`, `const` and `for` bind a name, `_`, or a tuple of those; \
                         use `match` to test a value",
                    ),
                );
                IrrefutablePatternKind::Wildcard
            }
        };
        IrrefutablePattern {
            id: pattern.id,
            kind,
            span,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::SourceMap;
    use crate::lexer;

    // ---- harness -----------------------------------------------------------

    fn parse_source(source: &str) -> (SourceUnit, Diagnostics) {
        let mut sources = SourceMap::new();
        let file = sources.add("test.pump", source);
        let mut diagnostics = Diagnostics::new();
        let tokens = lexer::tokenize(file, source, &mut diagnostics);
        let mut ids = NodeIdAllocator::new();
        let unit = parse(
            file,
            vec!["test".to_string()],
            &tokens,
            &mut ids,
            &mut diagnostics,
        );
        (unit, diagnostics)
    }

    fn summary(diagnostics: &Diagnostics) -> Vec<String> {
        diagnostics
            .entries()
            .iter()
            .map(|entry| format!("{}: {}", entry.code, entry.message))
            .collect()
    }

    fn parse_clean(source: &str) -> SourceUnit {
        let (unit, diagnostics) = parse_source(source);
        assert!(
            diagnostics.is_empty(),
            "expected a clean parse, got {:?}",
            summary(&diagnostics)
        );
        unit
    }

    fn codes(source: &str) -> Vec<ErrorCode> {
        parse_source(source)
            .1
            .entries()
            .iter()
            .map(|entry| entry.code)
            .collect()
    }

    fn function<'a>(unit: &'a SourceUnit, name: &str) -> &'a FunctionDecl {
        unit.declarations
            .iter()
            .find_map(|declaration| match declaration {
                Declaration::Function(decl) if decl.name.name == name => Some(decl),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no function named `{name}`"))
    }

    fn statements(source: &str) -> Vec<Stmt> {
        let program = format!("fn main() {{\n{source}\n}}\n");
        let unit = parse_clean(&program);
        function(&unit, "main").body.statements.clone()
    }

    fn expression(source: &str) -> Expr {
        let program = format!("fn main() {{\n    let value = {source}\n}}\n");
        let unit = parse_clean(&program);
        match &function(&unit, "main").body.statements[0].kind {
            StmtKind::Let(declaration) => declaration.value.clone(),
            other => panic!("expected a `let`, got {other:?}"),
        }
    }

    fn shape(expr: &Expr) -> String {
        match &expr.kind {
            ExprKind::Int(value) => value.to_string(),
            ExprKind::Float(value) => value.to_string(),
            ExprKind::Char(value) => format!("'{value}'"),
            ExprKind::Bool(value) => value.to_string(),
            ExprKind::Null => "null".to_string(),
            ExprKind::This => "this".to_string(),
            ExprKind::Ident(name) => name.name.clone(),
            ExprKind::Str(literal) => compound(
                "str",
                literal
                    .parts
                    .iter()
                    .map(|part| match part {
                        StringPart::Text { value, .. } => format!("{value:?}"),
                        StringPart::Interp(inner) => shape(inner),
                    })
                    .collect(),
            ),
            ExprKind::Array(items) => compound("array", shapes(items)),
            ExprKind::Set(items) => compound("set", shapes(items)),
            ExprKind::Tuple(items) => compound("tuple", shapes(items)),
            ExprKind::Map(entries) => compound(
                "map",
                entries
                    .iter()
                    .map(|entry| format!("{}:{}", shape(&entry.key), shape(&entry.value)))
                    .collect(),
            ),
            ExprKind::Group(inner) => format!("(group {})", shape(inner)),
            ExprKind::Closure(closure) => format!("(closure/{})", closure.params.len()),
            ExprKind::Unary { op, operand } => {
                let spelling = match op {
                    UnaryOp::Not => "!",
                    UnaryOp::Neg => "-",
                };
                format!("({spelling} {})", shape(operand))
            }
            ExprKind::Binary { op, lhs, rhs } => {
                format!("({} {} {})", op.spelling(), shape(lhs), shape(rhs))
            }
            ExprKind::Range {
                start,
                end,
                inclusive,
            } => {
                let spelling = if *inclusive { "..=" } else { ".." };
                format!("({spelling} {} {})", shape(start), shape(end))
            }
            ExprKind::Catch { operand, handler } => {
                let tail = match handler {
                    CatchHandler::Discard(_) => "discard".to_string(),
                    CatchHandler::Bind { name, .. } => format!("bind:{}", name.name),
                    CatchHandler::Value(value) => shape(value),
                };
                format!("(catch {} {tail})", shape(operand))
            }
            ExprKind::Field { base, name } => format!("(. {} {})", shape(base), name.name),
            ExprKind::TupleField { base, index, .. } => format!("(. {} {index})", shape(base)),
            ExprKind::Call { callee, args } => {
                let mut parts = vec![shape(callee)];
                parts.extend(args.iter().map(|argument| match &argument.name {
                    Some(name) => format!("{}:{}", name.name, shape(&argument.value)),
                    None => shape(&argument.value),
                }));
                compound("call", parts)
            }
            ExprKind::Index { base, index } => {
                format!("(index {} {})", shape(base), shape(index))
            }
            ExprKind::NullPropagate(inner) => format!("(? {})", shape(inner)),
            ExprKind::ErrorPropagate(inner) => format!("(! {})", shape(inner)),
            ExprKind::TypeArgs { base, args } => {
                format!("(turbofish {} {})", shape(base), args.len())
            }
            ExprKind::StructLit(literal) => {
                let mut parts = vec![literal.path.name.name.clone()];
                parts.extend(
                    literal
                        .fields
                        .iter()
                        .map(|field| format!("{}:{}", field.name.name, shape(&field.value))),
                );
                compound("struct", parts)
            }
        }
    }

    fn shapes(items: &[Expr]) -> Vec<String> {
        items.iter().map(shape).collect()
    }

    fn compound(head: &str, parts: Vec<String>) -> String {
        if parts.is_empty() {
            format!("({head})")
        } else {
            format!("({head} {})", parts.join(" "))
        }
    }

    // ---- file structure (grammar 10) ---------------------------------------

    #[test]
    fn a_semicolon_terminates_a_statement() {
        // muc 1 cua spec viet `let a = 10; let b = 20`, ma `term` trong
        // grammar la `( ";" | inserted_terminator )+`.
        let unit = parse_clean(
            "fn main() {
    let a = 10; let b = 20
}
",
        );
        assert_eq!(unit.declarations.len(), 1);
    }

    #[test]
    fn a_semicolon_separates_declaration_members() {
        parse_clean(
            "enum Color { Red; Green; Blue }
",
        );
        parse_clean(
            "struct Point { x: int; y: int }
",
        );
    }

    #[test]
    fn repeated_semicolons_are_empty_statements() {
        parse_clean(
            "fn main() {
    ;;
    let a = 1;;
}
",
        );
    }

    #[test]
    fn parses_the_specification_program() {
        let unit = parse_clean(
            r#"
import io
import net\http

const PORT: int = 8080

struct User {
    name: string
    age: int

    fn greet() {
        io.println("Hello " + name)
    }
}

fn create_user(name: string, age: int): User {
    let user = User {
        name: name
        age: age
    }

    return user
}

fn main() {
    let user = create_user("Minh", 18)

    if user.age >= 18 {
        user.greet()
    } else {
        io.println("Minor")
    }

    for i in 0..10 {
        io.println(i)
    }

    return
}
"#,
        );

        assert_eq!(unit.imports.len(), 2);
        assert_eq!(unit.imports[1].bound_name().name, "http");
        assert_eq!(unit.declarations.len(), 4);
        assert_eq!(function(&unit, "main").body.statements.len(), 4);
    }

    #[test]
    fn an_import_binds_its_last_segment_or_its_alias() {
        let unit = parse_clean("import net\\http\nimport net\\tcp as sockets\n");
        assert_eq!(unit.imports[0].path.len(), 2);
        assert_eq!(unit.imports[0].bound_name().name, "http");
        assert_eq!(unit.imports[1].bound_name().name, "sockets");
    }

    #[test]
    fn an_import_after_a_declaration_is_reported() {
        assert_eq!(
            codes("fn main() {\n}\nimport io\n"),
            vec![ErrorCode::ImportAfterDeclaration]
        );
    }

    #[test]
    fn two_imports_may_not_bind_the_same_name() {
        assert_eq!(
            codes("import io\nimport net\\io\n"),
            vec![ErrorCode::DuplicateImportBinding]
        );
    }

    #[test]
    fn module_level_state_must_be_const() {
        assert_eq!(codes("let x = 1\n"), vec![ErrorCode::TopLevelLet]);
    }

    #[test]
    fn visibility_is_recorded_and_may_not_be_repeated() {
        let unit = parse_clean("pub fn start() {\n}\nprivate fn internal() {\n}\n");
        assert!(function(&unit, "start").visibility.is_public());
        assert!(!function(&unit, "internal").visibility.is_public());
        assert_eq!(
            codes("pub private fn start() {\n}\n"),
            vec![ErrorCode::DuplicateVisibility]
        );
    }

    #[test]
    fn implements_lists_every_interface() {
        let unit = parse_clean("implements User: Printable, io.Writable\n");
        let Declaration::Implements(declaration) = &unit.declarations[0] else {
            panic!("expected an `implements`");
        };
        assert_eq!(declaration.subject.name, "User");
        assert_eq!(declaration.interfaces.len(), 2);
        assert_eq!(
            declaration.interfaces[1].module.as_ref().unwrap().name,
            "io"
        );
        assert_eq!(
            codes("pub implements User: Printable\n"),
            vec![ErrorCode::VisibilityOnImplements]
        );
    }

    // ---- declarations (grammar 12) -----------------------------------------

    #[test]
    fn a_struct_keeps_fields_and_methods_in_source_order() {
        let unit = parse_clean(
            "struct User {\n    name: string\n    fn greet() {\n    }\n    age: int\n}\n",
        );
        let Declaration::Struct(declaration) = &unit.declarations[0] else {
            panic!("expected a struct");
        };
        let names: Vec<&str> = declaration
            .members
            .iter()
            .map(|member| member.name().name.as_str())
            .collect();
        assert_eq!(names, ["name", "greet", "age"]);
        assert_eq!(declaration.fields().count(), 2);
        assert_eq!(declaration.methods().count(), 1);
    }

    #[test]
    fn a_function_typed_field_is_still_a_field() {
        let unit = parse_clean("struct Handler {\n    callback: fn(int): int\n}\n");
        let Declaration::Struct(declaration) = &unit.declarations[0] else {
            panic!("expected a struct");
        };
        assert_eq!(declaration.fields().count(), 1);
        assert_eq!(declaration.methods().count(), 0);
    }

    #[test]
    fn struct_members_are_not_separated_by_commas() {
        assert_eq!(
            codes("struct User {\n    name: string,\n    age: int\n}\n"),
            vec![ErrorCode::CommaBetweenStructMembers]
        );
    }

    #[test]
    fn an_enum_carries_variants_payloads_and_methods() {
        let unit = parse_clean(
            "enum Result<T, E> {\n    Ok(T)\n    Err(E)\n\n    fn describe(): string {\n        return \"\"\n    }\n}\n",
        );
        let Declaration::Enum(declaration) = &unit.declarations[0] else {
            panic!("expected an enum");
        };
        assert_eq!(declaration.generics.len(), 2);
        assert_eq!(declaration.variants().count(), 2);
        assert_eq!(declaration.methods().count(), 1);
        assert_eq!(declaration.variants().next().unwrap().payload.len(), 1);
    }

    #[test]
    fn a_payload_free_variant_is_written_without_parentheses() {
        assert_eq!(
            codes("enum Color {\n    Red()\n}\n"),
            vec![ErrorCode::EmptyVariantPayload]
        );
        assert_eq!( codes("enum Nothing {\n}\n"), vec![ErrorCode::EnumWithoutVariants] );
    }

    #[test]
    fn interface_methods_are_signatures_only() {
        let unit =
            parse_clean("interface Printable {\n    fn print()\n    fn label(): string\n}\n");
        let Declaration::Interface(declaration) = &unit.declarations[0] else {
            panic!("expected an interface");
        };
        assert_eq!(declaration.methods.len(), 2);
        assert!(declaration.methods[1].return_type.is_some());
        assert_eq!(
            codes("interface Printable {\n    fn print() {\n    }\n}\n"),
            vec![ErrorCode::InterfaceMethodHasBody]
        );
    }

    #[test]
    fn parameters_may_be_required_defaulted_or_variadic() {
        let unit = parse_clean("fn connect(host: string, port: int = 80, tags: ...string) {\n}\n");
        let params = &function(&unit, "connect").params;
        assert!(matches!(params[0].kind, ParamKind::Required));
        assert!(matches!(params[1].kind, ParamKind::Default(_)));
        assert!(matches!(params[2].kind, ParamKind::Variadic));
    }

    #[test]
    fn parameter_order_is_enforced() {
        assert_eq!( codes("fn f(a: int = 1, b: int) {\n}\n"), vec![ErrorCode::RequiredParameterAfterDefault] );
        assert_eq!(
            codes("fn f(a: ...int, b: int) {\n}\n"),
            vec![ErrorCode::VariadicNotLast]
        );
        assert_eq!(
            codes("fn f(a: ...int, b: ...int) {\n}\n"),
            vec![ErrorCode::MultipleVariadicParameters]
        );
        assert_eq!(
            codes("fn f(a: ...int = 1) {\n}\n"),
            vec![ErrorCode::VariadicWithDefault]
        );
    }

    #[test]
    fn generic_parameters_take_interface_bounds() {
        let unit = parse_clean(
            "fn first<T: Printable + Ordered, U>(items: [T]): T? {\n    return null\n}\n",
        );
        let generics = &function(&unit, "first").generics;
        assert_eq!(generics.len(), 2);
        assert_eq!(generics[0].bounds.len(), 2);
        assert!(generics[1].bounds.is_empty());
    }

    // ---- types (grammar 11) ------------------------------------------------

    fn binding_type(source: &str) -> TypeExpr {
        let program = format!("fn main() {{\n    let value: {source} = null\n}}\n");
        let unit = parse_clean(&program);
        match &function(&unit, "main").body.statements[0].kind {
            StmtKind::Let(declaration) => declaration.ty.clone().expect("an annotation"),
            other => panic!("expected a `let`, got {other:?}"),
        }
    }

    #[test]
    fn every_type_form_parses() {
        assert!(matches!(
            binding_type("int").kind,
            TypeExprKind::Path { .. }
        ));
        assert!(matches!(binding_type("[int]").kind, TypeExprKind::Array(_)));
        assert!(matches!(
            binding_type("[string: User]").kind,
            TypeExprKind::Map { .. }
        ));
        assert!(matches!(
            binding_type("set<int>").kind,
            TypeExprKind::Set(_)
        ));
        assert!(matches!(
            binding_type("(int, int)").kind,
            TypeExprKind::Tuple(_)
        ));
        assert!(matches!(
            binding_type("fn(int, ...string): bool").kind,
            TypeExprKind::Function(_)
        ));
        assert!(matches!(
            binding_type("User?").kind,
            TypeExprKind::Optional(_)
        ));
        assert!(matches!(binding_type("(int)").kind, TypeExprKind::Group(_)));
        assert!(matches!(
            binding_type("io.Writer").kind,
            TypeExprKind::Path { .. }
        ));
    }

    #[test]
    fn type_suffixes_apply_left_to_right() {
        // NOTE 11.5: `T?!` la "loi cua optional", khong phai nguoc lai.
        let ty = binding_type("string?!");
        let TypeExprKind::Failable(inner) = &ty.kind else {
            panic!("expected a failable type");
        };
        assert!(matches!(inner.kind, TypeExprKind::Optional(_)));
    }

    #[test]
    fn nested_type_arguments_split_the_closing_angle_brackets() {
        // 9.2: maximal munch cho ra `>>` mot token, phai tach no ra.
        let ty = binding_type("Box<Box<int>>");
        let TypeExprKind::Path { args, .. } = &ty.kind else {
            panic!("expected a path type");
        };
        assert_eq!(args.len(), 1);
        assert!(matches!(args[0].kind, TypeExprKind::Path { .. }));
        assert!(matches!(
            binding_type("set<set<int>>").kind,
            TypeExprKind::Set(_)
        ));
    }

    #[test]
    fn a_split_leaves_the_remainder_for_the_next_token() {
        // `Box<Box<int>>= v` quet thanh `>>` roi `=`, nen danh sach ngoai
        // an mot dau `>`, con lai dung cai `=` cua khoi tao.
        parse_clean("fn main() {\n    let b: Box<Box<int>>= v\n}\n");
    }

    #[test]
    fn a_bare_error_return_type_points_at_void() {
        assert_eq!(
            codes("fn f(): ! {\n}\n"),
            vec![ErrorCode::BareErrorReturnType]
        );
    }

    // ---- statements (grammar 13) -------------------------------------------

    #[test]
    fn if_else_if_else_chains() {
        let statements = statements(
            "    if score >= 90 {\n        f()\n    } else if score >= 80 {\n        g()\n    } else {\n        h()\n    }",
        );
        let StmtKind::If(outer) = &statements[0].kind else {
            panic!("expected an `if`");
        };
        let Some(ElseBranch::If(inner)) = &outer.else_branch else {
            panic!("expected an `else if`");
        };
        assert!(matches!(inner.else_branch, Some(ElseBranch::Block(_))));
    }

    #[test]
    fn else_may_start_on_its_own_line() {
        // 8.3: `else` nam trong bo bo qua, nen terminator vua chen sau `}`
        // bi vut di.
        statements("    if a() {\n    }\n    else {\n    }");
    }

    #[test]
    fn while_and_for_headers() {
        let statements =
            statements("    while i < 10 {\n        f()\n    }\n    for i in 0..=10 {\n        g()\n    }\n    for item in items {\n        h()\n    }");
        assert!(matches!(statements[0].kind, StmtKind::While(_)));
        let StmtKind::For(loop_over_range) = &statements[1].kind else {
            panic!("expected a `for`");
        };
        assert!(matches!(
            loop_over_range.iterable.kind,
            ExprKind::Range {
                inclusive: true,
                ..
            }
        ));
        let StmtKind::For(loop_over_items) = &statements[2].kind else {
            panic!("expected a `for`");
        };
        assert!(matches!(loop_over_items.iterable.kind, ExprKind::Ident(_)));
    }

    #[test]
    fn break_continue_return_and_fail() {
        let statements =
            statements("    break\n    continue\n    return\n    return 1\n    fail \"boom\"");
        assert!(matches!(statements[0].kind, StmtKind::Break));
        assert!(matches!(statements[1].kind, StmtKind::Continue));
        assert!(matches!(statements[2].kind, StmtKind::Return(None)));
        assert!(matches!(statements[3].kind, StmtKind::Return(Some(_))));
        assert!(matches!(statements[4].kind, StmtKind::Fail(_)));
    }

    #[test]
    fn a_statement_beginning_with_a_brace_is_a_block() {
        // 13.0.1: khong bao gio la map literal.
        let statements = statements("    {\n        f()\n    }");
        assert!(matches!(statements[0].kind, StmtKind::Block(_)));
    }

    #[test]
    fn assignment_covers_names_fields_and_elements() {
        let statements =
            statements("    count = 10\n    count += 1\n    user.age = 19\n    items[0] = 2\n    this.name = n");
        for statement in &statements {
            assert!(matches!(statement.kind, StmtKind::Assign(_)));
        }
        let StmtKind::Assign(compound) = &statements[1].kind else {
            panic!("expected an assignment");
        };
        assert_eq!(compound.op, AssignOp::Add);
    }

    #[test]
    fn a_call_is_not_an_assignment_target() {
        assert_eq!(
            codes("fn main() {\n    f() = 1\n}\n"),
            vec![ErrorCode::InvalidAssignmentTarget]
        );
    }

    #[test]
    fn assignment_is_not_an_expression() {
        assert_eq!(
            codes("fn main() {\n    a = b = c\n}\n"),
            vec![ErrorCode::AssignmentInExpression]
        );
        assert_eq!(
            codes("fn main() {\n    if a = b {\n    }\n}\n"),
            vec![ErrorCode::AssignmentInExpression]
        );
    }

    #[test]
    fn an_expression_statement_must_contain_a_call() {
        // 13.2.1 / D-5.
        for source in ["a + b", "x", "user.name"] {
            assert_eq!(
                codes(&format!("fn main() {{\n    {source}\n}}\n")),
                vec![ErrorCode::StatementHasNoEffect],
                "for `{source}`"
            );
        }
        statements("    print(x)\n    user.greet()\n    items.map(f).filter(g)");
    }

    #[test]
    fn a_named_function_may_not_be_nested() {
        assert_eq!(
            codes("fn main() {\n    fn helper() {\n    }\n}\n"),
            vec![ErrorCode::NestedFunctionDeclaration]
        );
    }

    // ---- match (grammar 13.4) ----------------------------------------------

    #[test]
    fn match_accepts_expression_arms_and_block_arms() {
        let statements = statements(
            "    match value {\n        0 => print(\"zero\")\n        1 => print(\"one\")\n        _ => print(\"other\")\n    }\n    match color {\n        Color.Red => {\n            print(\"red\")\n        }\n\n        Color.Green => {\n            print(\"green\")\n        }\n\n        _ => {\n            print(\"other\")\n        }\n    }",
        );
        let StmtKind::Match(by_value) = &statements[0].kind else {
            panic!("expected a `match`");
        };
        assert_eq!(by_value.arms.len(), 3);
        assert!(matches!(by_value.arms[0].body, MatchArmBody::Stmt(_)));
        assert!(matches!(
            by_value.arms[2].pattern.kind,
            PatternKind::Wildcard
        ));

        let StmtKind::Match(by_variant) = &statements[1].kind else {
            panic!("expected a `match`");
        };
        assert_eq!(by_variant.arms.len(), 3);
        assert!(matches!(by_variant.arms[0].body, MatchArmBody::Block(_)));
        assert!(matches!(
            by_variant.arms[0].pattern.kind,
            PatternKind::Variant { .. }
        ));
    }

    #[test]
    fn match_arms_may_be_separated_by_commas() {
        let statements = statements(
            "    match value {\n        0 => f(),\n        1 => g(),\n        _ => h(),\n    }",
        );
        let StmtKind::Match(declaration) = &statements[0].kind else {
            panic!("expected a `match`");
        };
        assert_eq!(declaration.arms.len(), 3);
    }

    #[test]
    fn match_arms_take_guards_and_or_patterns() {
        let statements = statements(
            "    match value {\n        1 | 2 | 3 => f()\n        n if n > 10 => g()\n        _ => h()\n    }",
        );
        let StmtKind::Match(declaration) = &statements[0].kind else {
            panic!("expected a `match`");
        };
        let PatternKind::Or(alternatives) = &declaration.arms[0].pattern.kind else {
            panic!("expected an or-pattern");
        };
        assert_eq!(alternatives.len(), 3);
        assert!(declaration.arms[1].guard.is_some());
        assert!(matches!(
            declaration.arms[1].pattern.kind,
            PatternKind::Binding(_)
        ));
    }

    // ---- patterns (grammar 15) ---------------------------------------------

    #[test]
    fn every_pattern_form_parses() {
        // may nhanh phai ngan bang dau phay vi `-` nam trong bo bo qua cua
        // 8.3: dong nao bat dau bang `-` la noi tiep dong truoc, nen nhanh co
        // so am can mot dau tach ma newline khong the cho.
        let statements = statements(
            "    match value {
        _ => a(),
        null => b(),
        true => c(),
        -1 => d(),
        'x' => e(),
        \"text\" => f(),
        1..=9 => g(),
        'a'..'z' => h(),
        Result.Ok(inner, _) => i(),
        User { name: n, .. } => j(),
        (left, right) => k(),
        rest => l(),
    }",
        );
        let StmtKind::Match(declaration) = &statements[0].kind else {
            panic!("expected a `match`");
        };
        let kinds: Vec<&PatternKind> = declaration
            .arms
            .iter()
            .map(|arm| &arm.pattern.kind)
            .collect();
        assert!(matches!(kinds[0], PatternKind::Wildcard));
        assert!(matches!(kinds[1], PatternKind::Null));
        assert!(matches!(kinds[2], PatternKind::Bool(true)));
        assert!(matches!(
            kinds[3],
            PatternKind::Int {
                magnitude: 1,
                negative: true
            }
        ));
        assert!(matches!(kinds[4], PatternKind::Char('x')));
        assert!(matches!(kinds[5], PatternKind::Str(_)));
        assert!(matches!(
            kinds[6],
            PatternKind::Range {
                inclusive: true,
                ..
            }
        ));
        assert!(matches!(
            kinds[7],
            PatternKind::Range {
                inclusive: false,
                ..
            }
        ));
        let PatternKind::Variant { payload, .. } = kinds[8] else {
            panic!("expected a variant pattern");
        };
        assert_eq!(payload.as_ref().map(Vec::len), Some(2));
        let PatternKind::Struct { fields, rest, .. } = kinds[9] else {
            panic!("expected a struct pattern");
        };
        assert_eq!(fields.len(), 1);
        assert!(rest);
        assert!(matches!(kinds[10], PatternKind::Tuple(_)));
        assert!(matches!(kinds[11], PatternKind::Binding(_)));
    }

    #[test]
    fn a_struct_pattern_needs_no_parentheses_in_a_match_header() {
        // NOTE 15.9: cho cua pattern khong phai cho cua bieu thuc.
        statements("    match x {\n        User { .. } => f()\n    }");
    }

    #[test]
    fn floats_and_interpolations_are_not_patterns() {
        assert_eq!(
            codes("fn main() {\n    match x {\n        1.5 => f()\n    }\n}\n"),
            vec![ErrorCode::FloatLiteralPattern]
        );
        assert_eq!(
            codes("fn main() {\n    match x {\n        \"{a}\" => f()\n    }\n}\n"),
            vec![ErrorCode::InterpolationInPattern]
        );
    }

    #[test]
    fn bindings_take_only_irrefutable_patterns() {
        let statements =
            statements("    let (a, b) = point\n    let _ = f()\n    for (k, v) in m {\n    }");
        let StmtKind::Let(destructuring) = &statements[0].kind else {
            panic!("expected a `let`");
        };
        assert_eq!(destructuring.pattern.bindings().len(), 2);
        assert_eq!( codes("fn main() {\n let Color.Red = x\n}\n"), vec![ErrorCode::RefutablePatternInBinding] );
    }

    // ---- expressions (grammar 14, precedence.md) ---------------------------

    #[test]
    fn the_precedence_table_worked_examples() {
        // dung tung dong cua bang o cuoi grammar/precedence.md.
        let cases = [
            ("a + b * c", "(+ a (* b c))"),
            ("a - b - c", "(- (- a b) c)"),
            ("-a * b", "(* (- a) b)"),
            ("-x.y", "(- (. x y))"),
            ("!a && b", "(&& (! a) b)"),
            ("a & b == c", "(== (& a b) c)"),
            ("a | b & c", "(| a (& b c))"),
            ("a + b << 2", "(<< (+ a b) 2)"),
            ("a == b && c == d", "(&& (== a b) (== c d))"),
            ("a && b || c", "(|| (&& a b) c)"),
            ("0..n - 1", "(.. 0 (- n 1))"),
            ("0..items.length", "(.. 0 (. items length))"),
            (
                "read(p) catch \"none\"",
                "(catch (call read p) (str \"none\"))",
            ),
            ("a + b catch 0", "(catch (+ a b) 0)"),
            ("x! + 1", "(+ (! x) 1)"),
            ("f()!.g()!", "(! (call (. (! (call f)) g)))"),
            ("x != y", "(!= x y)"),
            ("0..10", "(.. 0 10)"),
        ];
        for (source, expected) in cases {
            assert_eq!(shape(&expression(source)), expected, "for `{source}`");
        }
    }

    #[test]
    fn postfix_operators_apply_left_to_right() {
        // NOTE 14.17.
        assert_eq!(shape(&expression("user?.name")), "(. (? user) name)");
        assert_eq!(shape(&expression("a[0]?.b!")), "(! (. (? (index a 0)) b))");
        assert_eq!(shape(&expression("t.0.1")), "(. (. t 0) 1)");
    }

    #[test]
    fn comparison_and_range_are_non_associative() {
        assert_eq!(
            codes("fn main() {\n    let x = a < b < c\n}\n"),
            vec![ErrorCode::ChainedComparison]
        );
        assert_eq!(
            codes("fn main() {\n    let x = a..b..c\n}\n"),
            vec![ErrorCode::ChainedRange]
        );
    }

    #[test]
    fn explicit_type_arguments_use_turbofish() {
        // NOTE 14.16: trong bieu thuc thi `<` luon luon la so sanh.
        assert_eq!(
            shape(&expression("zero::<int>()")),
            "(call (turbofish zero 1))"
        );
        assert_eq!(shape(&expression("a < b")), "(< a b)");
    }

    #[test]
    fn every_literal_form_parses() {
        assert_eq!(shape(&expression("[1, 2]")), "(array 1 2)");
        assert_eq!(shape(&expression("[]")), "(array)");
        assert_eq!(shape(&expression("{}")), "(map)");
        assert_eq!(shape(&expression("{name: user}")), "(map name:user)");
        assert_eq!(shape(&expression("set{}")), "(set)");
        assert_eq!(shape(&expression("set{1, 2}")), "(set 1 2)");
        assert_eq!(shape(&expression("(a, b)")), "(tuple a b)");
        assert_eq!(shape(&expression("(a)")), "(group a)");
        assert_eq!(shape(&expression("3.14")), "3.14");
        assert_eq!(shape(&expression("'a'")), "'a'");
        assert_eq!(shape(&expression("null")), "null");
    }

    #[test]
    fn there_is_no_one_tuple_and_no_empty_tuple() {
        assert_eq!(
            codes("fn main() {\n    let x = (a,)\n}\n"),
            vec![ErrorCode::OneTupleNotAllowed]
        );
        assert_eq!(
            codes("fn main() {\n    let x = ()\n}\n"),
            vec![ErrorCode::EmptyTupleNotAllowed]
        );
    }

    #[test]
    fn struct_literals_accept_newlines_or_commas() {
        // NOTE 14.14: viet nhieu dong khong dau phay va viet mot dong co
        // dau phay ra cung mot literal.
        let multiline = expression("User {\n        name: \"Minh\"\n        age: 18\n    }");
        assert_eq!(
            shape(&multiline),
            "(struct User name:(str \"Minh\") age:18)"
        );
        let single = expression("User { name: \"Minh\", age: 18 }");
        assert_eq!(shape(&single), "(struct User name:(str \"Minh\") age:18)");
    }

    #[test]
    fn a_struct_literal_may_be_qualified_or_carry_a_turbofish() {
        let qualified = expression("models.User { name: n }");
        let ExprKind::StructLit(literal) = &qualified.kind else {
            panic!("expected a struct literal");
        };
        assert_eq!(literal.path.module.as_ref().unwrap().name, "models");

        let generic = expression("Box::<int> { value: 1 }");
        let ExprKind::StructLit(literal) = &generic.kind else {
            panic!("expected a struct literal");
        };
        assert_eq!(literal.type_args.len(), 1);
    }

    #[test]
    fn a_call_or_index_is_never_a_struct_literal_path() {
        // E-13: cai `{` do ket thuc bieu thuc chu khong mo them gi.
        let statements = statements("    f()\n    {\n        g()\n    }");
        assert!(matches!(statements[0].kind, StmtKind::Expr(_)));
        assert!(matches!(statements[1].kind, StmtKind::Block(_)));
    }

    #[test]
    fn calls_take_positional_and_named_arguments() {
        assert_eq!(
            shape(&expression("connect(\"localhost\", port: 8080)")),
            "(call connect (str \"localhost\") port:8080)"
        );
        assert_eq!(
            codes("fn main() {\n    connect(port: 8080, \"localhost\")\n}\n"),
            vec![ErrorCode::PositionalAfterNamed]
        );
    }

    #[test]
    fn closures_carry_typed_parameters_and_a_body() {
        assert_eq!(
            shape(&expression(
                "fn(a: int, b: int): int {\n        return a + b\n    }"
            )),
            "(closure/2)"
        );
        assert_eq!(
            shape(&expression(
                "items.map(fn(x: int): int {\n        return x * 2\n    })"
            )),
            "(call (. items map) (closure/1))"
        );
    }

    #[test]
    fn string_interpolation_holds_full_expressions() {
        assert_eq!(
            shape(&expression("\"Ten: {name}, age: {a + b}\"")),
            "(str \"Ten: \" name \", age: \" (+ a b))"
        );
    }

    #[test]
    fn catch_has_three_forms() {
        assert_eq!(
            shape(&expression("load() catch {\n        return\n    }")),
            "(catch (call load) discard)"
        );
        assert_eq!(
            shape(&expression("load() catch e {\n        return\n    }")),
            "(catch (call load) bind:e)"
        );
        assert_eq!(
            shape(&expression("load() catch 0")),
            "(catch (call load) 0)"
        );
        // 14.5: ket hop trai.
        assert_eq!(
            shape(&expression("load() catch 0 catch 1")),
            "(catch (catch (call load) 0) 1)"
        );
    }

    // ---- mode `ns` (grammar 9.1) -------------------------------------------

    #[test]
    fn a_bare_struct_literal_in_a_header_is_reported_with_the_required_help() {
        let (_, diagnostics) = parse_source(
            "fn main() {\n    if user == User { name: \"x\" } {\n        f()\n    }\n}\n",
        );
        let entries = diagnostics.entries();
        assert_eq!(entries.len(), 1, "{:?}", summary(&diagnostics));
        assert_eq!(entries[0].code, ErrorCode::StructLiteralInHeader);
        assert_eq!(
            entries[0].message,
            "struct literals are not allowed in the header of if / while / for / match"
        );
        assert!(entries[0].helps[0].contains("wrap it in parentheses"));
    }

    #[test]
    fn parenthesising_the_struct_literal_fixes_it() {
        statements("    if user == (User { name: \"x\" }) {\n        f()\n    }");
    }

    #[test]
    fn a_header_brace_is_the_block_not_a_literal() {
        // truong hop thuong gap phai im: `flag` la path va sau no la `{`.
        statements("    if flag {\n        f()\n    }\n    while ready {\n    }\n    for item in items {\n    }");
    }

    #[test]
    fn a_set_literal_is_exempt_from_the_header_rule() {
        // E-32: `set` la tu khoa nen no khong mo duoc block.
        statements("    while set{1, 2}.has(x) {\n        f()\n    }");
    }

    #[test]
    fn a_map_literal_may_not_open_a_header_either() {
        assert_eq!(
            codes("fn main() {\n    while {1: 2}.length > 0 {\n    }\n}\n"),
            vec![ErrorCode::StructLiteralInHeader]
        );
        statements("    while ({1: 2}).length > 0 {\n    }");
    }

    #[test]
    fn a_struct_literal_after_catch_is_reported_with_its_own_help() {
        let (_, diagnostics) =
            parse_source("fn main() {\n    let c = load() catch Config { retries: 1 }\n}\n");
        let entries = diagnostics.entries();
        assert_eq!(entries.len(), 1, "{:?}", summary(&diagnostics));
        assert_eq!(entries[0].code, ErrorCode::StructLiteralInHeader);
        assert!(entries[0].helps[0].contains("catch (Config"));
    }

    #[test]
    fn mode_ns_is_cleared_inside_brackets_and_call_arguments() {
        statements("    if contains(items, User { name: \"x\" }) {\n        f()\n    }");
        statements("    if flags[Key { id: 1 }] {\n        f()\n    }");
    }

    // ---- recovery ----------------------------------------------------------

    #[test]
    fn one_pass_reports_several_errors() {
        let found =
            codes("fn main() {\n    let x = (a,)\n    a + b\n    let y = c < d < e\n    f()\n}\n");
        assert_eq!(
            found,
            vec![
                ErrorCode::OneTupleNotAllowed,
                ErrorCode::StatementHasNoEffect,
                ErrorCode::ChainedComparison,
            ]
        );
    }

    #[test]
    fn a_broken_statement_does_not_swallow_the_next_one() {
        let (unit, diagnostics) = parse_source(
            "fn main() {
    let x = 1 2
    f()
}
",
        );
        assert_eq!(diagnostics.error_count(), 1, "{:?}", summary(&diagnostics));
        let body = &function(&unit, "main").body;
        assert!(matches!(body.statements[0].kind, StmtKind::Let(_)));
        assert!(matches!(body.statements[1].kind, StmtKind::Expr(_)));
    }

    #[test]
    fn a_broken_declaration_does_not_swallow_the_next_one() {
        let (unit, diagnostics) = parse_source("struct {\n}\n\nfn main() {\n    f()\n}\n");
        assert!(!diagnostics.is_empty());
        assert!(unit
            .declarations
            .iter()
            .any(|declaration| declaration.name().is_some_and(|name| name.name == "main")));
    }

    #[test]
    fn every_node_carries_a_real_span() {
        let unit = parse_clean("fn main() {\n    let value = 1 + 2\n}\n");
        let main = function(&unit, "main");
        assert!(!main.span.is_synthetic());
        assert_eq!(main.span.line, 1);
        let StmtKind::Let(declaration) = &main.body.statements[0].kind else {
            panic!("expected a `let`");
        };
        assert_eq!(declaration.span.line, 2);
        assert!(declaration.value.span.start < declaration.value.span.end);
    }

    #[test]
    fn an_import_path_must_lie_on_one_line() {
        assert_eq!(
            codes("import net\\\n    http\n"),
            vec![ErrorCode::MultilineImportPath]
        );
    }

    #[test]
    fn interpolations_nest() {
        // NOTE 3.4.4.
        assert_eq!(
            shape(&expression("\"{f(\"{x}\")}\"")),
            "(str (call f (str x)))"
        );
    }

    #[test]
    fn the_whole_language_parses_in_one_file() {
        let unit = parse_clean(
            r#"
import io
import net\http as web

const MAX_USERS: int = 1_000
pub const NAMES: [string] = ["a", "b"]

interface Printable {
    fn print()
}

enum Shape {
    Circle(float)
    Rect(float, float)

    fn area(): float {
        match this {
            Shape.Circle(r) => {
                return r * r
            }
            Shape.Rect(w, h) => {
                return w * h
            }
        }
        return 0.0
    }
}

pub struct Box<T> {
    pub value: T
    tag: string?
    onChange: fn(T): void

    pub fn get(): T {
        return this.value
    }

    fn rename(tag: string) {
        this.tag = tag
    }
}

implements Box: Printable

fn first<T: Printable>(items: [T]): T? {
    if items.length == 0 {
        return null
    }
    return items[0]
}

fn read_file(path: string): string! {
    if path == "" {
        fail "empty path"
    }
    return path
}

fn load(): string! {
    let data = read_file("data.txt")!
    let fallback = read_file("other.txt") catch "none"
    let handled = read_file("third.txt") catch error {
        return ""
    }
    return data + fallback + handled
}

fn main() {
    let numbers: [int] = [1, 2, 3, 4]
    let users: [string: Box<int>] = {}
    let ids: set<int> = set{}
    let point: (int, int) = (10, 20)
    let (x, y) = point
    let nested: Box<Box<int>> = Box::<Box<int>> { value: zero::<Box<int>>() }
    let add = fn(a: int, b: int): int {
        return a + b
    }

    let total = 0
    for i in 0..=10 {
        if i % 2 == 0 {
            continue
        } else if i > 8 {
            break
        }
        total += add(i, 1)
    }

    for name in users {
        io.println("name: {name}, total: {total}")
    }

    while total < MAX_USERS && !done {
        total = total * 2 + 1
    }

    let maybe = first(numbers)
    let value = maybe?
    io.println("{value}")
    web.serve(host: "localhost", port: 8080)

    match total {
        0 => io.println("zero")
        1 | 2 => io.println("small")
        3..=9 => io.println("medium")
        n if n > 100 => io.println("large")
        _ => {
            io.println("other")
        }
    }

    return
}
"#,
        );

        assert_eq!(unit.imports.len(), 2);
        assert_eq!(unit.declarations.len(), 10);
        assert_eq!(function(&unit, "main").body.statements.len(), 17);
    }
}
