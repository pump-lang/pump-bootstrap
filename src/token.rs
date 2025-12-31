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

impl TokenKind {
    /// Turn a word into a keyword kind. None if it is a normal ident.
    pub fn from_word(word: &str) -> Option<TokenKind> {
        use TokenKind::*;
        let kind = match word {
            // 27 tu khoa, 2.3.1. Thu tu abc cho de tim.
            "as" => As,
            "break" => Break,
            "catch" => Catch,
            "const" => Const,
            "continue" => Continue,
            "else" => Else,
            "enum" => Enum,
            "fail" => Fail,
            "false" => False,
            "fn" => Fn,
            "for" => For,
            "if" => If,
            "implements" => Implements,
            "import" => Import,
            "in" => In,
            "interface" => Interface,
            "let" => Let,
            "match" => Match,
            "null" => Null,
            "private" => Private,
            "pub" => Pub,
            "return" => Return,
            "set" => Set,
            "struct" => Struct,
            "this" => This,
            "true" => True,
            "while" => While,

            // may tu danh rieng co kind rieng
            "async" => Async,
            "defer" => Defer,

            // con lai, 2.4.1
            "await" | "channel" | "do" | "dyn" | "extern" | "finally" | "go" | "impl" | "loop"
            | "macro" | "module" | "mut" | "new" | "package" | "ref" | "select" | "self"
            | "spawn" | "static" | "switch" | "throw" | "trait" | "try" | "type" | "typeof"
            | "unsafe" | "use" | "var" | "where" | "yield" => ReservedWord,

            _ => return None,
        };
        Some(kind)
    }

    /// True for the kinds kept for a later Pump. Parser must refuse them
    /// with that exact message.
    pub fn is_reserved(self) -> bool {
        matches!(
            self,
            TokenKind::Async | TokenKind::Defer | TokenKind::At | TokenKind::ReservedWord
        )
    }

    /// True for the 27 keywords, grammar 2.3.1.
    pub fn is_keyword(self) -> bool {
        use TokenKind::*;
        matches!(
            self,
            As | Break
                | Catch
                | Const
                | Continue
                | Else
                | Enum
                | Fail
                | False
                | Fn
                | For
                | If
                | Implements
                | Import
                | In
                | Interface
                | Let
                | Match
                | Null
                | Private
                | Pub
                | Return
                | Set
                | Struct
                | This
                | True
                | While
        )
    }

    /// Bo dong cua 8.2: sau may kind nay thi newline thanh Terminator.
    pub fn is_closer(self) -> bool {
        use TokenKind::*;
        matches!(
            self,
            Ident
                | Underscore
                | IntLit
                | FloatLit
                | CharLit
                | TupleIndex
                | StringEnd
                | True
                | False
                | Null
                | This
                | Return
                | Break
                | Continue
                | RParen
                | RBracket
                | RBrace
                | Question
                | Bang
        )
    }

    /// Bo bo qua cua 8.3: kind khong the mo dau mot statement, nen
    /// terminator vua chen truoc no phai bo di.
    pub fn elides_terminator(self) -> bool {
        use TokenKind::*;
        matches!(
            self,
            Else | Catch
                | Dot
                | Comma
                | RParen
                | RBracket
                | RBrace
                | Colon
                | FatArrow
                | ColonColon
                | Plus
                | Minus
                | Star
                | Slash
                | Percent
                | EqEq
                | BangEq
                | Lt
                | Gt
                | LtEq
                | GtEq
                | AmpAmp
                | PipePipe
                | Amp
                | Pipe
                | Caret
                | Shl
                | Shr
                | DotDot
                | DotDotEq
                | Eq
                | PlusEq
                | MinusEq
                | StarEq
                | SlashEq
                | PercentEq
        )
    }

    /// True for the six assignment operators. Chi co sau cai, khong hon.
    pub fn is_assignment_operator(self) -> bool {
        use TokenKind::*;
        matches!(self, Eq | PlusEq | MinusEq | StarEq | SlashEq | PercentEq)
    }

    /// Opening brackets that the terminator stack of 8.1 follows.
    pub fn is_open_bracket(self) -> bool {
        use TokenKind::*;
        matches!(self, LParen | LBracket | LBrace | InterpStart)
    }

    /// The closing bracket that matches an opening one.
    pub fn closing_bracket(self) -> Option<TokenKind> {
        use TokenKind::*;
        match self {
            LParen => Some(RParen),
            LBracket => Some(RBracket),
            LBrace => Some(RBrace),
            InterpStart => Some(InterpEnd),
            _ => None,
        }
    }

    /// How to spell the token inside an error message.
    pub fn describe(self) -> &'static str {
        use TokenKind::*;
        match self {
            Ident => "an identifier",
            Underscore => "`_`",
            IntLit => "an integer literal",
            FloatLit => "a float literal",
            CharLit => "a character literal",
            TupleIndex => "a tuple index",
            StringStart => "the start of a string literal",
            StringText => "string text",
            InterpStart => "the start of an interpolation",
            InterpEnd => "the end of an interpolation",
            StringEnd => "the end of a string literal",
            As => "`as`",
            Break => "`break`",
            Catch => "`catch`",
            Const => "`const`",
            Continue => "`continue`",
            Else => "`else`",
            Enum => "`enum`",
            Fail => "`fail`",
            False => "`false`",
            Fn => "`fn`",
            For => "`for`",
            If => "`if`",
            Implements => "`implements`",
            Import => "`import`",
            In => "`in`",
            Interface => "`interface`",
            Let => "`let`",
            Match => "`match`",
            Null => "`null`",
            Private => "`private`",
            Pub => "`pub`",
            Return => "`return`",
            Set => "`set`",
            Struct => "`struct`",
            This => "`this`",
            True => "`true`",
            While => "`while`",
            Defer => "`defer`",
            At => "`@`",
            Async => "`async`",
            ReservedWord => "a reserved word",
            Plus => "`+`",
            Minus => "`-`",
            Star => "`*`",
            Slash => "`/`",
            Percent => "`%`",
            EqEq => "`==`",
            BangEq => "`!=`",
            Lt => "`<`",
            Gt => "`>`",
            LtEq => "`<=`",
            GtEq => "`>=`",
            AmpAmp => "`&&`",
            PipePipe => "`||`",
            Bang => "`!`",
            Amp => "`&`",
            Pipe => "`|`",
            Caret => "`^`",
            Shl => "`<<`",
            Shr => "`>>`",
            Eq => "`=`",
            PlusEq => "`+=`",
            MinusEq => "`-=`",
            StarEq => "`*=`",
            SlashEq => "`/=`",
            PercentEq => "`%=`",
            DotDot => "`..`",
            DotDotEq => "`..=`",
            Dot => "`.`",
            Question => "`?`",
            FatArrow => "`=>`",
            ColonColon => "`::`",
            Ellipsis => "`...`",
            LParen => "`(`",
            RParen => "`)`",
            LBracket => "`[`",
            RBracket => "`]`",
            LBrace => "`{`",
            RBrace => "`}`",
            Comma => "`,`",
            Colon => "`:`",
            Semicolon => "`;`",
            Backslash => "a backslash",
            Terminator => "a statement terminator",
            Eof => "end of file",
        }
    }
}

impl fmt::Display for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.describe())
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

// ===== test =====
//
// may cai test dau tien t viet trong ca repo. `cargo test token::` la ra.

#[cfg(test)]
mod tests {
    use super::*;

    fn sp(start: u32, end: u32) -> Span {
        Span::new(FileId(0), start, end, 1, start + 1)
    }

    #[test]
    fn to_gop_hai_span_lam_mot() {
        let a = sp(3, 7);
        let b = sp(10, 12);
        let c = a.to(b);
        assert_eq!(c.start, 3);
        assert_eq!(c.end, 12);
        // dong voi cot lay cua cai BEN TRAI, khong phai cua cai to hon
        assert_eq!(c.line, 1);
        assert_eq!(c.column, 4);
    }

    #[test]
    fn to_nguoc_thu_tu_van_ra_dung() {
        let a = sp(10, 12);
        let b = sp(3, 7);
        assert_eq!(a.to(b).start, 3);
        assert_eq!(a.to(b).end, 12);
    }

    #[test]
    fn start_point_rong_va_dung_cho() {
        let s = sp(4, 9);
        assert_eq!(s.len(), 5);
        assert!(!s.is_empty());
        assert!(s.start_point().is_empty());
        assert_eq!(s.start_point().start, 4);
        assert_eq!(s.start_point().end, 4);
    }

    #[test]
    fn synthetic_thi_biet_la_synthetic() {
        assert!(Span::synthetic().is_synthetic());
        assert!(!sp(0, 1).is_synthetic());
    }

    #[test]
    fn from_word_tra_dung_kind() {
        assert_eq!(TokenKind::from_word("fn").unwrap(), TokenKind::Fn);
        assert_eq!(TokenKind::from_word("match").unwrap(), TokenKind::Match);
        // hai cai nay danh rieng cho ban sau nhung co kind rieng, xem TODO.txt
        assert_eq!(TokenKind::from_word("defer").unwrap(), TokenKind::Defer);
        assert_eq!(TokenKind::from_word("async").unwrap(), TokenKind::Async);
        assert_eq!(
            TokenKind::from_word("spawn").unwrap(),
            TokenKind::ReservedWord
        );
        assert!(TokenKind::from_word("fizzbuzz").is_none());
        // Pump phan biet hoa thuong, nen IF chi la mot cai ten
        assert!(TokenKind::from_word("IF").is_none());
    }

    #[test]
    fn danh_rieng_thi_is_reserved() {
        assert!(TokenKind::Defer.is_reserved());
        assert!(TokenKind::At.is_reserved());
        assert!(TokenKind::Async.is_reserved());
        assert!(!TokenKind::Fn.is_reserved());
    }

    #[test]
    fn token_giu_lai_cai_ten() {
        let tk = Token::with_value(
            TokenKind::Ident,
            sp(0, 3),
            TokenValue::Ident("abc".to_string()),
        );
        assert!(tk.is(TokenKind::Ident));
        assert_eq!(tk.ident().unwrap(), "abc");
        assert!(Token::new(TokenKind::Comma, sp(3, 4)).ident().is_none());
    }

    #[test]
    fn describe_co_chu_cho_moi_kind() {
        // khong test het duoc, lay may cai hay hien trong thong bao loi
        assert_eq!(TokenKind::Eof.describe(), "end of file");
        assert_eq!(TokenKind::Comma.describe(), "`,`");
        assert!(!TokenKind::Terminator.describe().is_empty());
    }
}
