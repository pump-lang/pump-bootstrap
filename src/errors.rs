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
