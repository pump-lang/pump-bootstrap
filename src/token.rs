// token va span.
//
// moi token deu deo mot Span de bao loi con biet chi vao dau. Dung vut span
// di va cung dung noi rong no ra, sau nay t con dinh viet formatter tren cai
// nay nua.

#![allow(dead_code)]

use std::fmt;

/// Id of one source file inside the SourceMap.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct FileId(pub u32);

impl FileId {
    /// File id for span that does not come from real source.
    pub const SYNTHETIC: FileId = FileId(u32::MAX);

    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Byte range [start, end) in one file, plus line and column of start so
/// the error printer does not have to count again.
/// column is counted in BYTES, khong phai ky tu.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Span {
    pub file: FileId,
    pub start: u32,
    pub end: u32,
    pub line: u32,
    pub column: u32,
}

impl Span {
    pub fn new(file: FileId, start: u32, end: u32, line: u32, column: u32) -> Span {
        Span {
            file,
            start,
            end,
            line,
            column,
        }
    }

    /// A span pointing at nothing. Dung cho node compiler tu che ra.
    pub fn synthetic() -> Span {
        Span {
            file: FileId::SYNTHETIC,
            start: 0,
            end: 0,
            line: 1,
            column: 1,
        }
    }

    pub fn is_synthetic(self) -> bool {
        self.file == FileId::SYNTHETIC
    }

    /// Smallest span that covers both.
    pub fn to(self, other: Span) -> Span {
        Span {
            file: self.file,
            start: self.start.min(other.start),
            end: self.end.max(other.end),
            line: self.line,
            column: self.column,
        }
    }

    /// Zero width span at the start, for "expected X here".
    pub fn start_point(self) -> Span {
        Span {
            end: self.start,
            ..self
        }
    }

    pub fn len(self) -> u32 {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(self) -> bool {
        self.len() == 0
    }
}

/// One token: kind, span, and the value if it has one.
#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
    pub value: TokenValue,
}

impl Token {
    pub fn new(kind: TokenKind, span: Span) -> Token {
        Token {
            kind,
            span,
            value: TokenValue::None,
        }
    }

    pub fn with_value(kind: TokenKind, span: Span, value: TokenValue) -> Token {
        Token { kind, span, value }
    }

    pub fn is(&self, kind: TokenKind) -> bool {
        self.kind == kind
    }

    /// Text of the identifier, only for Ident and ReservedWord.
    pub fn ident(&self) -> Option<&str> {
        match &self.value {
            TokenValue::Ident(name) => Some(name),
            _ => None,
        }
    }
}

/// What a token carries beside its kind.
#[derive(Clone, Debug, PartialEq)]
pub enum TokenValue {
    None,
    Ident(String),
    Int(u64),
    Float(f64),
    Char(char),
    Str(String),
}

/// Every kind of token the lexer can make.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TokenKind {
    // ---- ten va literal (2.2, 3) ----
    Ident,
    Underscore,
    IntLit,
    FloatLit,
    CharLit,
    TupleIndex,

    // ---- 1 chuoi la mot day token chu khong phai 1 token (3.4) ----
    StringStart,
    StringText,
    InterpStart,
    InterpEnd,
    StringEnd,

    // ---- 27 tu khoa (2.3.1) ----
    As,
    Break,
    Catch,
    Const,
    Continue,
    Else,
    Enum,
    Fail,
    False,
    Fn,
    For,
    If,
    Implements,
    Import,
    In,
    Interface,
    Let,
    Match,
    Null,
    Private,
    Pub,
    Return,
    Set,
    Struct,
    This,
    True,
    While,

    // ---- de danh cho ban sau (2.4) ----
    //
    // lexer nhan ra may cai nay nhung parser khong bao gio xu ly, chi bao
    // "reserved for a future version of Pump" roi thoi. Defer, At, Async co
    // kind rieng vi cu phap cua chung phac ra roi, con lai deu ve ReservedWord
    // kem text trong TokenValue::Ident.
    Defer,
    At,
    Async,
    ReservedWord,

    // ---- toan tu (4) ----
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    EqEq,
    BangEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    AmpAmp,
    PipePipe,
    Bang,
    Amp,
    Pipe,
    Caret,
    Shl,
    Shr,
    Eq,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    DotDot,
    DotDotEq,
    Dot,
    Question,
    FatArrow,
    ColonColon,
    Ellipsis,

    // ---- dau cau (4) ----
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Colon,
    Semicolon,
    Backslash,

    // ---- tu che ra, khong co trong source ----
    Terminator,
    Eof,
}
