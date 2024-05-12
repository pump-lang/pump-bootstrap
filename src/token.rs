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
