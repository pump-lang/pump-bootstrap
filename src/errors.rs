// bao loi. Ma loi, CompileError, va cai ham ve ra file:line:col voi may
// dau mui ten nho nho o duoi.
//
// trong compiler khong cho cho nao in thang
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
