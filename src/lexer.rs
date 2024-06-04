// scanner. Chu vao, token ra.
//
// ba cho lam t mat thoi gian hon ca phan con lai cong lai:
//
//  * chen terminator (muc 8), nhin token truoc voi token sau, tat han o
//    trong `(`, `[` va o trong noi suy,
//  * che do chuoi (3.4.1), mot chuoi la mot DAY token chu khong phai mot
//    token, de parser dung lai duoc phan parse bieu thuc binh thuong o trong
//    cap `{}`,
//  * luat so voi dau cham (3.2.2), chinh no lam cho `0..10` va `1.max()` deu
//    quet ra dung cai ma minh nghi trong dau.
//
// Trong file nay khong co cho nao de quy. Ngoac, noi suy, chuoi long nhau,
// tat ca nam tren mot cai stack ro rang, nen file long sau ngu ngoc thi ton
// heap chu khong lam vo call stack.

#![allow(dead_code)]

use crate::errors::{CompileError, Diagnostics, ErrorCode};
use crate::token::{FileId, Span, Token, TokenKind, TokenValue};

// 128 la bua, gap bug thi tang len. Spec 3.4.4 chi doi it nhat 32.
const MAX_INTERPOLATION_DEPTH: usize = 128;

// dat tam vao cho literal nao giai ma khong ra, de day token con nguyen
// hinh dang va parser con di tiep qua cho bao loi duoc.
const REPLACEMENT: char = '\u{fffd}';

const ONLY_SIX_ASSIGNMENTS: &str =
    "Pump 1.0 has only the assignment operators `=` `+=` `-=` `*=` `/=` `%=`";

/// Scan the source into a token list. Cai cuoi cung luon la Eof.
pub fn tokenize(file: FileId, source: &str, diagnostics: &mut Diagnostics) -> Vec<Token> {
    Lexer::new(file, source, diagnostics).run()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FrameKind {
    Paren,
    Bracket,
    Brace,
    Interpolation,
    String,
}

impl FrameKind {
    fn describe(self) -> &'static str {
        match self {
            FrameKind::Paren => "this `(`",
            FrameKind::Bracket => "this `[`",
            FrameKind::Brace => "this `{`",
            FrameKind::Interpolation => "this interpolation",
            FrameKind::String => "this string literal",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Frame {
    kind: FrameKind,
    span: Span,
}

#[derive(Clone, Copy, Debug)]
struct Mark {
    pos: usize,
    line: u32,
    column: u32,
}

struct Lexer<'a> {
    file: FileId,
    source: &'a str,
    diagnostics: &'a mut Diagnostics,
    pos: usize,
    line: u32,
    line_start: usize,
    tokens: Vec<Token>,
    frames: Vec<Frame>,
    pending_terminator: Option<Span>,
    in_module_path: bool,
}

impl<'a> Lexer<'a> {
    fn new(file: FileId, source: &'a str, diagnostics: &'a mut Diagnostics) -> Lexer<'a> {
        Lexer {
            file,
            source,
            diagnostics,
            pos: 0,
            line: 1,
            line_start: 0,
            tokens: Vec::new(),
            frames: Vec::new(),
            pending_terminator: None,
            in_module_path: false,
        }
    }

    fn run(mut self) -> Vec<Token> {
        self.skip_byte_order_mark();
        loop {
            if self.in_string_body() {
                self.scan_string_body();
                continue;
            }
            if self.skip_trivia() {
                continue;
            }
            if self.pos >= self.source.len() {
                break;
            }
            let before = self.pos;
            self.scan_token();
            debug_assert!(self.pos > before, "scan_token must always make progress");
        }
        self.finish()
    }

    // ===== the input =====
    fn peek(&self) -> Option<u8> {
        self.source.as_bytes().get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.source.as_bytes().get(self.pos + offset).copied()
    }

    fn current_char(&self) -> Option<char> {
        self.source[self.pos..].chars().next()
    }

    fn advance(&mut self) {
        debug_assert!(self.peek().is_some_and(|byte| byte < 0x80));
        self.pos += 1;
    }

    fn advance_char(&mut self) {
        if let Some(character) = self.current_char() {
            self.pos += character.len_utf8();
        }
    }

    fn advance_line(&mut self) {
        debug_assert_eq!(self.peek(), Some(b'\n'));
        self.pos += 1;
        self.line += 1;
        self.line_start = self.pos;
    }

    fn column(&self) -> u32 {
        (self.pos - self.line_start) as u32 + 1
    }

    fn mark(&self) -> Mark {
        Mark {
            pos: self.pos,
            line: self.line,
            column: self.column(),
        }
    }

    fn span_from(&self, mark: &Mark) -> Span {
        Span::new(
            self.file,
            mark.pos as u32,
            self.pos as u32,
            mark.line,
            mark.column,
        )
    }

    fn span_at(&self, mark: &Mark, length: usize) -> Span {
        Span::new(
            self.file,
            mark.pos as u32,
            (mark.pos + length) as u32,
            mark.line,
            mark.column,
        )
    }

    fn point_span(&self) -> Span {
        Span::new(
            self.file,
            self.pos as u32,
            self.pos as u32,
            self.line,
            self.column(),
        )
    }

    fn skip_byte_order_mark(&mut self) {
        if self.source.starts_with('\u{feff}') {
            self.pos = '\u{feff}'.len_utf8();
            self.line_start = self.pos;
        }
    }

    // ===== diagnostics =====
    fn report(&mut self, error: CompileError) {
        self.diagnostics.push(error);
    }

    // ===== token sink =====
    fn last_kind(&self) -> Option<TokenKind> {
        self.tokens.last().map(|token| token.kind)
    }

    fn push(&mut self, token: Token) {
        if let Some(span) = self.pending_terminator.take() {
            if !token.kind.elides_terminator() {
                self.tokens.push(Token::new(TokenKind::Terminator, span));
                self.in_module_path = false;
            }
        }
        // 10.2.2: dang o duong dan module la sau `import`, roi mot day
        // identifier voi dau gach cheo nguoc. Gap token khac la thoat.
        self.in_module_path = match token.kind {
            TokenKind::Import => true,
            TokenKind::Ident | TokenKind::Backslash => self.in_module_path,
            _ => false,
        };
        self.tokens.push(token);
    }

    fn push_kind(&mut self, kind: TokenKind, span: Span) {
        self.push(Token::new(kind, span));
    }

    fn emit_fixed(&mut self, mark: &Mark, length: usize, kind: TokenKind) {
        self.pos = mark.pos + length;
        let span = self.span_from(mark);
        self.push_kind(kind, span);
    }

    fn reject_operator(&mut self, mark: &Mark, length: usize, message: &str) {
        self.pos = mark.pos + length;
        let span = self.span_from(mark);
        self.report(CompileError::at(
            ErrorCode::OperatorNotInPump,
            span,
            message,
        ));
    }

    fn finish(mut self) -> Vec<Token> {
        let eof_span = self.point_span();

        // 8.5: chen terminator o cuoi file neu token cuoi la dau dong.
        // Terminator dang cho thi khong con token nao de nuot no nua nen no
        // song sot.
        match self.pending_terminator.take() {
            Some(span) => self.tokens.push(Token::new(TokenKind::Terminator, span)),
            None => {
                if self.last_kind().is_some_and(TokenKind::is_closer) {
                    self.tokens
                        .push(Token::new(TokenKind::Terminator, eof_span));
                }
            }
        }

        for frame in std::mem::take(&mut self.frames) {
            let error = match frame.kind {
                FrameKind::String => CompileError::new(ErrorCode::UnterminatedString, frame.span)
                    .with_caret("this string literal is never closed"),
                kind => CompileError::at(
                    ErrorCode::UnclosedBracketAtEof,
                    frame.span,
                    format!("{} is never closed", kind.describe()),
                ),
            };
            self.report(error);
        }

        self.tokens.push(Token::new(TokenKind::Eof, eof_span));
        self.tokens
    }

    // ===== trivia =====

    // dau cach voi tab thi chi seperate token ra thoi, khong sinh token nao.
    // Xuong dong thi khac han: no di qua handle_newline vi con co the thanh
    // mot terminator.
    fn skip_trivia(&mut self) -> bool {
        let start = self.pos;
        while let Some(byte) = self.peek() {
            match byte {
                b' ' | b'\t' => self.advance(),
                b'\n' => {
                    let mark = self.mark();
                    self.advance_line();
                    let span = self.span_from(&mark);
                    self.handle_newline(span);
                }
                b'\r' => self.skip_carriage_return(),
                b'/' if self.peek_at(1) == Some(b'/') => self.skip_line_comment(),
                b'/' if self.peek_at(1) == Some(b'*') => self.skip_block_comment(),
                _ if byte >= 0x80 => {
                    let Some(character) = self.current_char() else {
                        break;
                    };
                    if !character.is_whitespace() {
                        break;
                    }
                    let mark = self.mark();
                    self.advance_char();
                    let span = self.span_from(&mark);
                    self.report(
                        CompileError::at(
                            ErrorCode::InvalidWhitespace,
                            span,
                            format!("`U+{:04X}` may not separate tokens", character as u32),
                        )
                        .with_note(
                            "only a space and a tab separate tokens, and only a line feed ends \
                             a line; an invisible character must never change what a program \
                             means",
                        ),
                    );
                }
                _ if byte < 0x20 || byte == 0x7f => {
                    let mark = self.mark();
                    self.advance();
                    let span = self.span_from(&mark);
                    self.report(CompileError::at(
                        ErrorCode::InvalidWhitespace,
                        span,
                        format!("`U+{byte:04X}` is a control character"),
                    ));
                }
                _ => break,
            }
        }
        self.pos != start
    }

    fn skip_carriage_return(&mut self) {
        if self.peek_at(1) == Some(b'\n') {
            self.advance();
            return;
        }
        let mark = self.mark();
        self.advance();
        let span = self.span_from(&mark);
        self.report(
            CompileError::new(ErrorCode::LoneCarriageReturn, span)
                .with_help("Pump accepts LF and CRLF line endings"),
        );
    }

    fn skip_line_comment(&mut self) {
        while let Some(byte) = self.peek() {
            if byte == b'\n' {
                break;
            }
            // duyet theo byte van an toan, byte noi tiep UTF-8 khong bao gio la ASCII
            self.pos += 1;
        }
    }

    fn skip_block_comment(&mut self) {
        let mark = self.mark();
        self.pos += 2;
        let mut depth = 1usize;
        let mut first_newline: Option<Span> = None;

        while depth > 0 {
            match self.peek() {
                None => {
                    let span = self.span_at(&mark, 2);
                    self.report(
                        CompileError::new(ErrorCode::UnterminatedBlockComment, span)
                            .with_caret("this comment is never closed")
                            .with_note("block comments nest, so every `/*` needs its own `*/`"),
                    );
                    return;
                }
                Some(b'/') if self.peek_at(1) == Some(b'*') => {
                    self.pos += 2;
                    depth += 1;
                }
                Some(b'*') if self.peek_at(1) == Some(b'/') => {
                    self.pos += 2;
                    depth -= 1;
                }
                Some(b'\n') => {
                    let newline = self.mark();
                    self.advance_line();
                    if first_newline.is_none() {
                        first_newline = Some(self.span_from(&newline));
                    }
                }
                Some(_) => self.pos += 1,
            }
        }

        if let Some(span) = first_newline {
            self.handle_newline(span);
        }
    }

    // ===== newline handling =====
    fn handle_newline(&mut self, span: Span) {
        // 3.4.3: khong cho xuong dong tho trong chuoi, ke ca trong phan noi
        // suy. Bao loi ngay o day thi phan chen terminator va phan quet chuoi
        // khong dinh vao nhau.
        if self
            .frames
            .iter()
            .any(|frame| frame.kind == FrameKind::String)
        {
            self.report(
                CompileError::new(ErrorCode::NewlineInString, span)
                    .with_help("write `\\n`, or split the text and join the parts with `+`"),
            );
            return;
        }

        // 8.1: dau ngoac trong cung dang mo se tat viec chen
        match self.frames.last().map(|frame| frame.kind) {
            Some(FrameKind::Paren | FrameKind::Bracket | FrameKind::Interpolation) => return,
            Some(FrameKind::Brace | FrameKind::String) | None => {}
        }

        // 10.2.3: duong dan import khong duoc viet tiep o dong sau
        if self.in_module_path && self.last_kind() == Some(TokenKind::Backslash) {
            self.report(
                CompileError::new(ErrorCode::MultilineImportPath, span)
                    .with_help("write the whole path on one line, as in `import net\\http`"),
            );
            return;
        }

        if self.pending_terminator.is_some() {
            return;
        }

        // 8.2: chen, quyet dinh theo token cuoi cung
        if self.last_kind().is_some_and(TokenKind::is_closer) {
            self.pending_terminator = Some(span);
            self.in_module_path = false;
        }
    }

    // ===== names =====
    fn scan_token(&mut self) {
        let byte = self.peek().unwrap();
        match byte {
            b'"' => self.scan_string_start(),
            b'\'' => self.scan_char_literal(),
            b'0'..=b'9' => {
                // 9.6: chu so ngay sau token `.` la chi so tuple, nho vay
                // `t.0.1` thanh hai lan truy cap chu khong phai so thuc
                if self.last_kind() == Some(TokenKind::Dot) {
                    self.scan_tuple_index();
                } else {
                    self.scan_number();
                }
            }
            _ if is_ident_start(byte) => self.scan_word(),
            _ => self.scan_operator(),
        }
    }

    fn scan_word(&mut self) {
        let mark = self.mark();
        while self.peek().is_some_and(is_ident_continue) {
            self.advance();
        }
        let text = &self.source[mark.pos..self.pos];
        let span = self.span_from(&mark);

        // 2.2.2: `_` la token wildcard chu khong phai identifier. `_x` voi
        // `__` van la identifier binh thuong.
        if text == "_" {
            self.push_kind(TokenKind::Underscore, span);
            return;
        }

        match TokenKind::from_word(text) {
            // tu danh rieng phai mang theo text de parser goi ten no ra duoc
            Some(kind @ TokenKind::ReservedWord) => {
                let value = TokenValue::Ident(text.to_string());
                self.push(Token::with_value(kind, span, value));
            }
            Some(kind) => self.push_kind(kind, span),
            None => {
                let value = TokenValue::Ident(text.to_string());
                self.push(Token::with_value(TokenKind::Ident, span, value));
            }
        }
    }

    // ===== numbers =====
    fn scan_number(&mut self) {
        let mark = self.mark();
        let mut digits = String::new();
        let mut separator_reported = false;

        let radix = self.scan_radix_prefix(&mark);
        if radix != 10 {
            self.pos = mark.pos + 2;
            let count = self.scan_digit_run(radix, &mut digits, &mut separator_reported);
            if count == 0 {
                let span = self.span_from(&mark);
                self.report(CompileError::at(
                    ErrorCode::MalformedNumericLiteral,
                    span,
                    format!("expected at least one {} digit", radix_name(radix)),
                ));
            }
            self.check_numeric_suffix(&mark, radix);
            let span = self.span_from(&mark);
            let value = self.integer_value(&digits, radix, span);
            self.push(Token::with_value(
                TokenKind::IntLit,
                span,
                TokenValue::Int(value),
            ));
            return;
        }

        self.scan_digit_run(10, &mut digits, &mut separator_reported);
        let mut is_float = false;

        // 3.2.1 va 3.2.2: so thuc phai co chu so o ca hai ben dau cham, the
        // nen `0..10` ra `0` `..` `10`, con `1.max()` ra `1` `.` `max` `(` `)`.
        // cho nay minh sai mat may hom moi ra.
        if self.peek() == Some(b'.') && self.peek_at(1).is_some_and(|byte| byte.is_ascii_digit()) {
            is_float = true;
            digits.push('.');
            self.advance();
            self.scan_digit_run(10, &mut digits, &mut separator_reported);
        }

        if self.scan_exponent(&mut digits, &mut separator_reported) {
            is_float = true;
        }

        self.check_numeric_suffix(&mark, 10);
        let span = self.span_from(&mark);

        if is_float {
            let value = digits.parse::<f64>().unwrap_or(0.0);
            if value.is_infinite() {
                self.report(CompileError::at(
                    ErrorCode::MalformedNumericLiteral,
                    span,
                    "this float literal is too large for `float`",
                ));
            }
            self.push(Token::with_value(
                TokenKind::FloatLit,
                span,
                TokenValue::Float(value),
            ));
        } else {
            let value = self.integer_value(&digits, 10, span);
            self.push(Token::with_value(
                TokenKind::IntLit,
                span,
                TokenValue::Int(value),
            ));
        }
    }

    fn scan_radix_prefix(&mut self, mark: &Mark) -> u32 {
        if self.peek() != Some(b'0') {
            return 10;
        }
        match self.peek_at(1) {
            Some(b'x') => 16,
            Some(b'o') => 8,
            Some(b'b') => 2,
            Some(byte @ (b'X' | b'O' | b'B')) => {
                let span = self.span_at(mark, 2);
                let lower = byte.to_ascii_lowercase() as char;
                self.report(
                    CompileError::at(
                        ErrorCode::MalformedNumericLiteral,
                        span,
                        "a radix prefix is written in lower case",
                    )
                    .with_help(format!("write `0{lower}`")),
                );
                match byte {
                    b'X' => 16,
                    b'O' => 8,
                    _ => 2,
                }
            }
            _ => 10,
        }
    }

    fn scan_exponent(&mut self, digits: &mut String, separator_reported: &mut bool) -> bool {
        if !matches!(self.peek(), Some(b'e' | b'E')) {
            return false;
        }

        let signed = matches!(self.peek_at(1), Some(b'+' | b'-'));
        let digit_offset = if signed { 2 } else { 1 };

        if !self
            .peek_at(digit_offset)
            .is_some_and(|byte| byte.is_ascii_digit())
        {
            let exponent = self.mark();
            self.advance();
            if signed {
                self.advance();
            }
            let span = self.span_from(&exponent);
            self.report(CompileError::at(
                ErrorCode::MalformedNumericLiteral,
                span,
                "an exponent needs at least one digit",
            ));
            return false;
        }

        digits.push('e');
        self.advance();
        if signed {
            let sign = self.peek().unwrap() as char;
            digits.push(sign);
            self.advance();
        }
        self.scan_digit_run(10, digits, separator_reported);
        true
    }

    fn scan_digit_run(&mut self, radix: u32, out: &mut String, reported: &mut bool) -> usize {
        let mut count = 0usize;
        let mut previous_was_digit = false;

        while let Some(byte) = self.peek() {
            if is_digit_for(byte, radix) {
                out.push(byte as char);
                count += 1;
                previous_was_digit = true;
                self.advance();
            } else if byte == b'_' {
                let next_is_digit = self
                    .peek_at(1)
                    .is_some_and(|next| is_digit_for(next, radix));
                if (!previous_was_digit || !next_is_digit) && !*reported {
                    *reported = true;
                    let mark = self.mark();
                    let span = self.span_at(&mark, 1);
                    self.report(
                        CompileError::new(ErrorCode::InvalidDigitSeparator, span).with_help(
                            "`1_000` and `0xFF_FF` are legal; `_1`, `1_` and `1__0` are not",
                        ),
                    );
                }
                previous_was_digit = false;
                self.advance();
            } else {
                break;
            }
        }
        count
    }

    fn check_numeric_suffix(&mut self, mark: &Mark, radix: u32) {
        let Some(byte) = self.peek() else {
            return;
        };
        if !is_ident_start(byte) && !byte.is_ascii_digit() {
            return;
        }

        let suffix = self.mark();
        while self.peek().is_some_and(is_ident_continue) {
            self.advance();
        }
        let span = self.span_from(&suffix);
        let text = self.source[suffix.pos..self.pos].to_string();
        let literal = self.source[mark.pos..suffix.pos].to_string();

        let message = if radix != 10 && text.bytes().all(|byte| byte.is_ascii_digit()) {
            format!("`{text}` is not a valid {} digit", radix_name(radix))
        } else {
            format!("`{literal}` is followed by `{text}`")
        };
        self.report(
            CompileError::at(ErrorCode::MalformedNumericLiteral, span, message).with_note(
                "Pump 1.0 has no literal type suffixes; a literal adopts its type from context",
            ),
        );
    }

    fn integer_value(&mut self, digits: &str, radix: u32, span: Span) -> u64 {
        if digits.is_empty() {
            return 0;
        }
        match u64::from_str_radix(digits, radix) {
            Ok(value) => value,
            Err(_) => {
                self.report(
                    CompileError::new(ErrorCode::IntegerLiteralTooLarge, span)
                        .with_note("the largest literal Pump can write is 18446744073709551615"),
                );
                0
            }
        }
    }

    fn scan_tuple_index(&mut self) {
        let mark = self.mark();
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.advance();
        }
        let digits = self.source[mark.pos..self.pos].to_string();

        if self.peek().is_some_and(is_ident_start) {
            let suffix = self.mark();
            while self.peek().is_some_and(is_ident_continue) {
                self.advance();
            }
            let span = self.span_from(&suffix);
            self.report(CompileError::at(
                ErrorCode::MalformedNumericLiteral,
                span,
                "a tuple index is a plain decimal number",
            ));
        }

        let span = self.span_from(&mark);
        let value = self.integer_value(&digits, 10, span);
        self.push(Token::with_value(
            TokenKind::TupleIndex,
            span,
            TokenValue::Int(value),
        ));
    }

    // ===== character literals =====
    fn scan_char_literal(&mut self) {
        let mark = self.mark();
        self.advance();
        let mut escape_failed = false;

        let value = match self.peek() {
            None => {
                let span = self.span_from(&mark);
                self.report(CompileError::new(ErrorCode::UnterminatedCharLiteral, span));
                self.push_char_literal(&mark, REPLACEMENT);
                return;
            }
            Some(b'\'') => {
                self.advance();
                let span = self.span_from(&mark);
                self.report(
                    CompileError::new(ErrorCode::EmptyCharLiteral, span)
                        .with_help("a character literal holds exactly one character, as in `'a'`"),
                );
                self.push_char_literal(&mark, REPLACEMENT);
                return;
            }
            // 3.3.2: xuong dong tho trong char la loi
            Some(b'\n' | b'\r') => {
                let span = self.span_from(&mark);
                self.report(
                    CompileError::new(ErrorCode::UnterminatedCharLiteral, span)
                        .with_help("write `'\\n'` for a line feed"),
                );
                self.push_char_literal(&mark, REPLACEMENT);
                return;
            }
            Some(b'\\') => match self.scan_escape() {
                Some(character) => character,
                None => {
                    escape_failed = true;
                    REPLACEMENT
                }
            },
            Some(_) => {
                let character = self.current_char().unwrap();
                self.advance_char();
                character
            }
        };

        if self.peek() == Some(b'\'') {
            self.advance();
            self.push_char_literal(&mark, value);
            return;
        }

        // chay toi dau nhay dong, nhung tuyet doi khong vuot qua dong
        let mut closed = false;
        while let Some(byte) = self.peek() {
            if byte == b'\n' || byte == b'\r' {
                break;
            }
            if byte == b'\'' {
                self.advance();
                closed = true;
                break;
            }
            self.advance_char();
        }

        let span = self.span_from(&mark);
        if closed {
            // escape bi tu choi thi scanner dang dung giua literal. Da bao
            // loi roi, bao them "too long" nua thanh hai loi cho mot cai sai.
            if !escape_failed {
                self.report(
                    CompileError::new(ErrorCode::CharLiteralTooLong, span)
                        .with_help("use a string literal for more than one character"),
                );
            }
        } else {
            self.report(CompileError::new(ErrorCode::UnterminatedCharLiteral, span));
        }
        self.push_char_literal(&mark, value);
    }

    fn push_char_literal(&mut self, mark: &Mark, value: char) {
        let span = self.span_from(mark);
        self.push(Token::with_value(
            TokenKind::CharLit,
            span,
            TokenValue::Char(value),
        ));
    }

    // ===== escapes =====
    fn scan_escape(&mut self) -> Option<char> {
        let mark = self.mark();
        self.advance();

        let Some(byte) = self.peek() else {
            let span = self.span_from(&mark);
            self.report(CompileError::at(
                ErrorCode::UnknownEscape,
                span,
                "a backslash must be followed by an escape character",
            ));
            return None;
        };

        let simple = match byte {
            b'n' => Some('\n'),
            b'r' => Some('\r'),
            b't' => Some('\t'),
            b'0' => Some('\0'),
            b'\\' => Some('\\'),
            b'"' => Some('"'),
            b'\'' => Some('\''),
            b'{' => Some('{'),
            b'}' => Some('}'),
            _ => None,
        };
        if let Some(character) = simple {
            self.advance();
            return Some(character);
        }

        match byte {
            b'x' => self.scan_ascii_escape(&mark),
            b'u' => self.scan_unicode_escape(&mark),
            _ => {
                self.advance_char();
                let span = self.span_from(&mark);
                let text = self.source[mark.pos..self.pos].to_string();
                self.report(
                    CompileError::at(
                        ErrorCode::UnknownEscape,
                        span,
                        format!("unknown escape sequence `{text}`"),
                    )
                    .with_note("Pump has \\n \\r \\t \\0 \\\\ \\\" \\' \\{ \\} \\xHH and \\u{...}"),
                );
                None
            }
        }
    }

    fn scan_ascii_escape(&mut self, mark: &Mark) -> Option<char> {
        self.advance();
        let mut value = 0u32;
        for _ in 0..2 {
            match self.peek().and_then(|byte| (byte as char).to_digit(16)) {
                Some(digit) => {
                    value = value * 16 + digit;
                    self.advance();
                }
                None => {
                    let span = self.span_from(mark);
                    self.report(CompileError::at(
                        ErrorCode::InvalidUnicodeEscape,
                        span,
                        "`\\x` must be followed by exactly two hexadecimal digits",
                    ));
                    return None;
                }
            }
        }
        if value > 0x7f {
            let span = self.span_from(mark);
            self.report(
                CompileError::new(ErrorCode::AsciiEscapeOutOfRange, span)
                    .with_help("use `\\u{...}` for a character above 0x7F"),
            );
            return None;
        }
        char::from_u32(value)
    }

    fn scan_unicode_escape(&mut self, mark: &Mark) -> Option<char> {
        self.advance();
        if self.peek() != Some(b'{') {
            let span = self.span_from(mark);
            self.report(CompileError::at(
                ErrorCode::InvalidUnicodeEscape,
                span,
                "`\\u` must be followed by `{`, as in `\\u{1F600}`",
            ));
            return None;
        }
        self.advance();

        let mut value = 0u32;
        let mut digits = 0usize;
        while digits < 6 {
            let Some(digit) = self.peek().and_then(|byte| (byte as char).to_digit(16)) else {
                break;
            };
            value = value * 16 + digit;
            digits += 1;
            self.advance();
        }

        if digits == 0 || self.peek() != Some(b'}') {
            if self.peek() == Some(b'}') {
                self.advance();
            }
            let span = self.span_from(mark);
            self.report(CompileError::at(
                ErrorCode::InvalidUnicodeEscape,
                span,
                "`\\u{...}` takes one to six hexadecimal digits",
            ));
            return None;
        }
        self.advance();

        match char::from_u32(value) {
            Some(character) => Some(character),
            None => {
                let span = self.span_from(mark);
                self.report(
                    CompileError::at(
                        ErrorCode::InvalidUnicodeEscape,
                        span,
                        format!("`U+{value:X}` is not a Unicode scalar value"),
                    )
                    .with_note("a scalar value is at most 0x10FFFF and outside 0xD800..=0xDFFF"),
                );
                None
            }
        }
    }

    // ===== string literals =====
    fn in_string_body(&self) -> bool {
        self.frames.last().map(|frame| frame.kind) == Some(FrameKind::String)
    }

    fn scan_string_start(&mut self) {
        let mark = self.mark();
        self.advance();
        let span = self.span_from(&mark);
        self.push_kind(TokenKind::StringStart, span);
        self.frames.push(Frame {
            kind: FrameKind::String,
            span,
        });
    }

    fn scan_string_body(&mut self) {
        let mark = self.mark();
        let mut text = String::new();

        loop {
            let run_start = self.pos;
            while let Some(byte) = self.peek() {
                if matches!(byte, b'"' | b'{' | b'\\' | b'\n' | b'\r') {
                    break;
                }
                // duyet theo byte van an toan, khong dau phan cach nao o tren
                // nam trong mot day UTF-8 nhieu byte ca
                self.pos += 1;
            }
            text.push_str(&self.source[run_start..self.pos]);

            match self.peek() {
                Some(b'\\') => {
                    if let Some(character) = self.scan_escape() {
                        text.push(character);
                    }
                }
                Some(b'\r') if self.peek_at(1) != Some(b'\n') => self.skip_carriage_return(),
                _ => break,
            }
        }

        if self.pos > mark.pos {
            let span = self.span_from(&mark);
            self.push(Token::with_value(
                TokenKind::StringText,
                span,
                TokenValue::Str(text),
            ));
        }

        match self.peek() {
            Some(b'"') => {
                let quote = self.mark();
                self.advance();
                let span = self.span_from(&quote);
                self.frames.pop();
                self.push_kind(TokenKind::StringEnd, span);
            }
            // 3.4.2: chi `{` khong bi escape moi mo noi suy, ma escape thi an
            // het o tren roi
            Some(b'{') => self.open_interpolation(),
            // xuong dong tho hoac het file. 3.4.3 cam cai dau, ma dang nao
            // chuoi cung chua dong; dong no lai o day de phan con lai cua file
            // van quet binh thuong.
            _ => self.close_unterminated_string(),
        }
    }

    fn open_interpolation(&mut self) {
        let mark = self.mark();
        self.advance();
        let span = self.span_from(&mark);

        let depth = self
            .frames
            .iter()
            .filter(|frame| frame.kind == FrameKind::Interpolation)
            .count();
        if depth == MAX_INTERPOLATION_DEPTH {
            self.report(
                CompileError::new(ErrorCode::InterpolationTooDeep, span).with_note(format!(
                    "at most {MAX_INTERPOLATION_DEPTH} levels are supported"
                )),
            );
        }

        self.push_kind(TokenKind::InterpStart, span);
        self.frames.push(Frame {
            kind: FrameKind::Interpolation,
            span,
        });
    }

    fn close_unterminated_string(&mut self) {
        let opening = self.frames.pop().unwrap();
        let span = self.point_span();

        let error = match self.peek() {
            Some(_) => CompileError::new(ErrorCode::NewlineInString, span)
                .with_secondary(opening.span, "this string literal starts here")
                .with_help("write `\\n`, or split the text and join the parts with `+`"),
            None => CompileError::new(ErrorCode::UnterminatedString, opening.span)
                .with_caret("this string literal is never closed"),
        };
        self.report(error);
        self.push_kind(TokenKind::StringEnd, span);
    }

    // ===== operators, brackets =====
    fn scan_operator(&mut self) {
        let mark = self.mark();
        let byte = self.peek().unwrap();
        let next = self.peek_at(1);

        match byte {
            b'+' => match next {
                Some(b'=') => self.emit_fixed(&mark, 2, TokenKind::PlusEq),
                Some(b'+') => {
                    self.reject_operator(&mark, 2, "Pump has no increment operator; use `+= 1`")
                }
                _ => self.emit_fixed(&mark, 1, TokenKind::Plus),
            },
            b'-' => match next {
                Some(b'=') => self.emit_fixed(&mark, 2, TokenKind::MinusEq),
                Some(b'>') => {
                    self.reject_operator(&mark, 2, "Pump writes a return type with `:`, not `->`")
                }
                Some(b'-') => {
                    self.reject_operator(&mark, 2, "Pump has no decrement operator; use `-= 1`")
                }
                _ => self.emit_fixed(&mark, 1, TokenKind::Minus),
            },
            b'*' => match next {
                Some(b'=') => self.emit_fixed(&mark, 2, TokenKind::StarEq),
                Some(b'*') => {
                    self.reject_operator(&mark, 2, "Pump has no exponent operator; use `math.pow`")
                }
                _ => self.emit_fixed(&mark, 1, TokenKind::Star),
            },
            // comment bi skip_trivia an het roi nen `/` o day la phep chia
            b'/' => match next {
                Some(b'=') => self.emit_fixed(&mark, 2, TokenKind::SlashEq),
                _ => self.emit_fixed(&mark, 1, TokenKind::Slash),
            },
            b'%' => match next {
                Some(b'=') => self.emit_fixed(&mark, 2, TokenKind::PercentEq),
                _ => self.emit_fixed(&mark, 1, TokenKind::Percent),
            },
            b'=' => match next {
                Some(b'=') => self.emit_fixed(&mark, 2, TokenKind::EqEq),
                Some(b'>') => self.emit_fixed(&mark, 2, TokenKind::FatArrow),
                _ => self.emit_fixed(&mark, 1, TokenKind::Eq),
            },
            b'!' => match next {
                Some(b'=') => self.emit_fixed(&mark, 2, TokenKind::BangEq),
                _ => self.emit_fixed(&mark, 1, TokenKind::Bang),
            },
            b'<' => {
                if next == Some(b'<') && self.peek_at(2) == Some(b'=') {
                    self.reject_operator(&mark, 3, ONLY_SIX_ASSIGNMENTS);
                } else {
                    match next {
                        Some(b'<') => self.emit_fixed(&mark, 2, TokenKind::Shl),
                        Some(b'=') => self.emit_fixed(&mark, 2, TokenKind::LtEq),
                        _ => self.emit_fixed(&mark, 1, TokenKind::Lt),
                    }
                }
            }
            // co y khong chan `>>=` o day. 9.2 bat `let b: Box<Box<int>>= v`
            // phai parse duoc, muon the thi parser phai nhan `>>` roi `=` va tu
            // tach lay `>>`.
            b'>' => match next {
                Some(b'>') => self.emit_fixed(&mark, 2, TokenKind::Shr),
                Some(b'=') => self.emit_fixed(&mark, 2, TokenKind::GtEq),
                _ => self.emit_fixed(&mark, 1, TokenKind::Gt),
            },
            b'&' => match next {
                Some(b'&') => self.emit_fixed(&mark, 2, TokenKind::AmpAmp),
                Some(b'=') => self.reject_operator(&mark, 2, ONLY_SIX_ASSIGNMENTS),
                _ => self.emit_fixed(&mark, 1, TokenKind::Amp),
            },
            b'|' => match next {
                Some(b'|') => self.emit_fixed(&mark, 2, TokenKind::PipePipe),
                Some(b'=') => self.reject_operator(&mark, 2, ONLY_SIX_ASSIGNMENTS),
                _ => self.emit_fixed(&mark, 1, TokenKind::Pipe),
            },
            b'^' => match next {
                Some(b'=') => self.reject_operator(&mark, 2, ONLY_SIX_ASSIGNMENTS),
                _ => self.emit_fixed(&mark, 1, TokenKind::Caret),
            },
            b'.' => match (next, self.peek_at(2)) {
                (Some(b'.'), Some(b'.')) => self.emit_fixed(&mark, 3, TokenKind::Ellipsis),
                (Some(b'.'), Some(b'=')) => self.emit_fixed(&mark, 3, TokenKind::DotDotEq),
                (Some(b'.'), _) => self.emit_fixed(&mark, 2, TokenKind::DotDot),
                _ => self.emit_fixed(&mark, 1, TokenKind::Dot),
            },
            b':' => match next {
                Some(b':') => self.emit_fixed(&mark, 2, TokenKind::ColonColon),
                _ => self.emit_fixed(&mark, 1, TokenKind::Colon),
            },
            // 4.3: khong co `?.` va khong co `??`, ca hai deu quet thanh token
            // roi, ma nghia cung dung nhu vay luon
            b'?' => self.emit_fixed(&mark, 1, TokenKind::Question),
            b',' => self.emit_fixed(&mark, 1, TokenKind::Comma),
            b';' => self.emit_fixed(&mark, 1, TokenKind::Semicolon),
            b'~' => self.reject_operator(&mark, 1, "Pump 1.0 has no bitwise NOT; use `x ^ -1`"),
            // 2.4: `@` de danh cho cu phap attribute sau nay. Scanner nhan ra,
            // parser bao loi, y het `defer` voi `async`.
            b'@' => self.emit_fixed(&mark, 1, TokenKind::At),
            b'#' => self.reject_operator(&mark, 1, "Pump 1.0 has no directive syntax"),
            b'$' => self.reject_operator(&mark, 1, "Pump 1.0 has no sigil syntax"),
            b'(' => self.open_bracket(&mark, FrameKind::Paren, TokenKind::LParen),
            b')' => self.close_bracket(&mark, FrameKind::Paren, TokenKind::RParen),
            b'[' => self.open_bracket(&mark, FrameKind::Bracket, TokenKind::LBracket),
            b']' => self.close_bracket(&mark, FrameKind::Bracket, TokenKind::RBracket),
            b'{' => self.open_bracket(&mark, FrameKind::Brace, TokenKind::LBrace),
            b'}' => self.close_brace(&mark),
            b'\\' => self.scan_backslash(&mark),
            _ => self.scan_unexpected(&mark),
        }
    }

    fn scan_backslash(&mut self, mark: &Mark) {
        if self.in_module_path {
            self.emit_fixed(mark, 1, TokenKind::Backslash);
            return;
        }
        self.pos = mark.pos + 1;
        let span = self.span_from(mark);
        self.report(
            CompileError::new(ErrorCode::BackslashOutsideImportPath, span).with_note(
                "a backslash is only valid inside a string literal and in an import path",
            ),
        );
    }

    fn scan_unexpected(&mut self, mark: &Mark) {
        let character = self.current_char().unwrap();
        self.advance_char();

        // 2.2.1: chu hoac so khong phai ASCII thi gan nhu chac chan la nguoi
        // ta dang co dat ten, noi thang the con hon "unexpected character".
        // Van nha token ra, mot cai ten sai khong lam hong ca lan parse.
        if character.is_alphanumeric() {
            while self
                .current_char()
                .is_some_and(|next| next.is_alphanumeric() || next == '_')
            {
                self.advance_char();
            }
            let span = self.span_from(mark);
            let text = self.source[mark.pos..self.pos].to_string();
            self.report(
                CompileError::new(ErrorCode::NonAsciiIdentifier, span)
                    .with_note("Pump 1.0 identifiers use `A`-`Z`, `a`-`z`, `0`-`9` and `_`"),
            );
            self.push(Token::with_value(
                TokenKind::Ident,
                span,
                TokenValue::Ident(text),
            ));
            return;
        }

        let span = self.span_from(mark);
        self.report(CompileError::at(
            ErrorCode::UnexpectedCharacter,
            span,
            format!("unexpected character `{character}`"),
        ));
    }

    fn open_bracket(&mut self, mark: &Mark, kind: FrameKind, token: TokenKind) {
        self.pos = mark.pos + 1;
        let span = self.span_from(mark);
        self.push_kind(token, span);
        self.frames.push(Frame { kind, span });
    }

    fn close_bracket(&mut self, mark: &Mark, kind: FrameKind, token: TokenKind) {
        self.pos = mark.pos + 1;
        let span = self.span_from(mark);
        if self.frames.last().map(|frame| frame.kind) == Some(kind) {
            self.frames.pop();
        } else {
            self.report(CompileError::at(
                ErrorCode::UnmatchedClosingBracket,
                span,
                format!("unmatched {}", token.describe()),
            ));
        }
        self.push_kind(token, span);
    }

    fn close_brace(&mut self, mark: &Mark) {
        if self.frames.last().map(|frame| frame.kind) != Some(FrameKind::Interpolation) {
            self.close_bracket(mark, FrameKind::Brace, TokenKind::RBrace);
            return;
        }

        self.pos = mark.pos + 1;
        let span = self.span_from(mark);
        let opening = self.frames.pop().unwrap();

        // 3.4.7: `"{}"` la loi. Noi suy rong thi ngay truoc token nay khong
        // the la gi khac ngoai token mo.
        if self.last_kind() == Some(TokenKind::InterpStart) {
            self.report(
                CompileError::at(
                    ErrorCode::EmptyInterpolation,
                    opening.span.to(span),
                    "an interpolation must contain an expression",
                )
                .with_help("write `\\{}` for a literal pair of braces"),
            );
        }

        self.push_kind(TokenKind::InterpEnd, span);
    }
}

fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_ident_continue(byte: u8) -> bool {
    is_ident_start(byte) || byte.is_ascii_digit()
}

fn is_digit_for(byte: u8, radix: u32) -> bool {
    (byte as char).to_digit(radix).is_some()
}

fn radix_name(radix: u32) -> &'static str {
    match radix {
        2 => "binary",
        8 => "octal",
        16 => "hexadecimal",
        _ => "decimal",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::TokenKind::*;

    fn scan(source: &str) -> (Vec<Token>, Vec<ErrorCode>) {
        let mut diagnostics = Diagnostics::new();
        let tokens = tokenize(FileId(0), source, &mut diagnostics);
        let codes = diagnostics
            .entries()
            .iter()
            .map(|error| error.code)
            .collect();
        (tokens, codes)
    }

    #[track_caller]
    fn kinds(source: &str) -> Vec<TokenKind> {
        let (tokens, codes) = scan(source);
        assert!(codes.is_empty(), "unexpected diagnostics {codes:?}");
        tokens.into_iter().map(|token| token.kind).collect()
    }

    #[track_caller]
    fn body(source: &str) -> Vec<TokenKind> {
        let mut kinds = kinds(source);
        assert_eq!(kinds.pop(), Some(Eof));
        if kinds.last() == Some(&Terminator) {
            kinds.pop();
        }
        kinds
    }

    #[track_caller]
    fn codes(source: &str) -> Vec<ErrorCode> {
        scan(source).1
    }

    #[track_caller]
    fn payloads(source: &str) -> Vec<TokenValue> {
        let (tokens, codes) = scan(source);
        assert!(codes.is_empty(), "unexpected diagnostics {codes:?}");
        tokens
            .into_iter()
            .map(|token| token.value)
            .filter(|value| !matches!(value, TokenValue::None))
            .collect()
    }

    #[track_caller]
    fn ints(source: &str) -> Vec<u64> {
        payloads(source)
            .into_iter()
            .filter_map(|value| match value {
                TokenValue::Int(number) => Some(number),
                _ => None,
            })
            .collect()
    }

    #[track_caller]
    fn floats(source: &str) -> Vec<f64> {
        payloads(source)
            .into_iter()
            .filter_map(|value| match value {
                TokenValue::Float(number) => Some(number),
                _ => None,
            })
            .collect()
    }

    #[track_caller]
    fn chars(source: &str) -> Vec<char> {
        payloads(source)
            .into_iter()
            .filter_map(|value| match value {
                TokenValue::Char(character) => Some(character),
                _ => None,
            })
            .collect()
    }

    #[track_caller]
    fn strings(source: &str) -> Vec<String> {
        payloads(source)
            .into_iter()
            .filter_map(|value| match value {
                TokenValue::Str(text) => Some(text),
                _ => None,
            })
            .collect()
    }

    // ===== names and keywords =====
    #[test]
    fn every_keyword_scans_as_its_own_kind() {
        let source = "as break catch const continue else enum fail false fn for if \
                      implements import in interface let match null private pub return \
                      set struct this true while";
        let scanned: Vec<TokenKind> = kinds(source)
            .into_iter()
            .filter(|kind| !matches!(kind, Terminator | Eof))
            .collect();
        assert_eq!(scanned.len(), 27, "grammar 2.3.1 lists 27 keywords");
        assert!(scanned.iter().all(|kind| kind.is_keyword()), "{scanned:?}");
    }

    #[test]
    fn reserved_words_are_recognised_but_never_treated_specially() {
        assert_eq!(
            body("defer async spawn channel type"),
            vec![Defer, Async, ReservedWord, ReservedWord, ReservedWord]
        );
        assert_eq!(body("@"), vec![At]);
        assert!(codes("defer async @ spawn").is_empty());
    }

    #[test]
    fn a_reserved_word_carries_its_spelling() {
        let (tokens, _) = scan("spawn");
        assert_eq!(tokens[0].ident(), Some("spawn"));
    }

    #[test]
    fn underscore_is_its_own_token_but_longer_names_are_identifiers() {
        assert_eq!(
            body("_ _x __ x1 _1"),
            vec![Underscore, Ident, Ident, Ident, Ident]
        );
    }

    #[test]
    fn identifiers_carry_their_text() {
        assert_eq!(
            payloads("alpha _beta g2"),
            vec![
                TokenValue::Ident("alpha".to_string()),
                TokenValue::Ident("_beta".to_string()),
                TokenValue::Ident("g2".to_string()),
            ]
        );
    }

    #[test]
    fn predeclared_names_are_ordinary_identifiers() {
        assert_eq!(body("int uint float string print len"), vec![Ident; 6]);
    }

    // ===== numbers =====
    #[test]
    fn integer_literals_in_every_radix() {
        assert_eq!(
            ints("42 0xFF 0o17 0b1010 1_000 0xFF_FF 010 0"),
            vec![42, 255, 15, 10, 1_000, 0xFFFF, 10, 0]
        );
    }

    #[test]
    fn the_largest_literal_fits_and_the_next_one_does_not() {
        assert_eq!(ints("18446744073709551615"), vec![u64::MAX]);
        assert_eq!(
            codes("18446744073709551616"),
            vec![ErrorCode::IntegerLiteralTooLarge]
        );
    }

    #[test]
    fn a_digit_separator_must_sit_between_two_digits() {
        assert_eq!(codes("1_"), vec![ErrorCode::InvalidDigitSeparator]);
        assert_eq!(codes("1__0"), vec![ErrorCode::InvalidDigitSeparator]);
        assert_eq!(codes("0x_1"), vec![ErrorCode::InvalidDigitSeparator]);
        assert_eq!(
            body("_1"),
            vec![Ident],
            "`_1` is an identifier, not a number"
        );
    }

    #[test]
    fn malformed_numbers_are_named_precisely() {
        assert_eq!(codes("0x"), vec![ErrorCode::MalformedNumericLiteral]);
        assert_eq!(codes("0b102"), vec![ErrorCode::MalformedNumericLiteral]);
        assert_eq!(codes("42i64"), vec![ErrorCode::MalformedNumericLiteral]);
        assert_eq!(codes("1e"), vec![ErrorCode::MalformedNumericLiteral]);
        assert_eq!(codes("0X1"), vec![ErrorCode::MalformedNumericLiteral]);
    }

    #[test]
    fn float_literals_and_exponents() {
        assert_eq!(
            floats("1.5 3.75 1e5 1E-3 2.5e+2 1_000.000_1"),
            vec![1.5, 3.75, 1e5, 1e-3, 2.5e2, 1_000.000_1]
        );
    }

    #[test]
    fn a_dot_joins_a_number_only_when_a_digit_follows() {
        assert_eq!(body("0..10"), vec![IntLit, DotDot, IntLit]);
        assert_eq!(body("0..=10"), vec![IntLit, DotDotEq, IntLit]);
        assert_eq!(body("1.max()"), vec![IntLit, Dot, Ident, LParen, RParen]);
        assert_eq!(body("1.5"), vec![FloatLit]);
        assert_eq!(ints("0..10"), vec![0, 10]);
    }

    #[test]
    fn digits_after_a_dot_token_are_a_tuple_index() {
        assert_eq!(body("t.0.1"), vec![Ident, Dot, TupleIndex, Dot, TupleIndex]);
        assert_eq!(ints("t.0.1"), vec![0, 1]);
    }

    // ===== character literals =====
    #[test]
    fn character_literals_and_their_escapes() {
        assert_eq!(
            chars(r"'a' '\n' '\t' '\0' '\\' '\'' '\x41' '\u{1F600}' 'e'"),
            vec!['a', '\n', '\t', '\0', '\\', '\'', 'A', '\u{1F600}', 'e']
        );
    }

    #[test]
    fn a_character_literal_holds_one_scalar_value() {
        assert_eq!(chars("'\u{e9}'"), vec!['\u{e9}']);
        assert_eq!(chars("'\u{65e5}'"), vec!['\u{65e5}']);
    }

    #[test]
    fn broken_character_literals_are_reported_and_still_produce_a_token() {
        assert_eq!(codes("''"), vec![ErrorCode::EmptyCharLiteral]);
        assert_eq!(codes("'ab'"), vec![ErrorCode::CharLiteralTooLong]);
        assert_eq!(codes("'a"), vec![ErrorCode::UnterminatedCharLiteral]);
        // 3.3.2: xuong dong tho ket thuc literal chu khong bi nuot
        assert_eq!(codes("'a\nb"), vec![ErrorCode::UnterminatedCharLiteral]);
        assert_eq!(codes(r"'\q'"), vec![ErrorCode::UnknownEscape]);
        assert_eq!(codes(r"'\x80'"), vec![ErrorCode::AsciiEscapeOutOfRange]);
        assert_eq!(codes(r"'\u{D800}'"), vec![ErrorCode::InvalidUnicodeEscape]);
        assert_eq!(
            codes(r"'\u{110000}'"),
            vec![ErrorCode::InvalidUnicodeEscape]
        );
        assert_eq!(codes(r"'\xZZ'"), vec![ErrorCode::InvalidUnicodeEscape]);

        let (tokens, _) = scan("''");
        assert_eq!(tokens[0].kind, CharLit);
    }

    // ===== string literals =====
    #[test]
    fn a_string_is_a_bracketed_token_sequence() {
        assert_eq!(body(r#""hello""#), vec![StringStart, StringText, StringEnd]);
        assert_eq!(body(r#""""#), vec![StringStart, StringEnd]);
        assert_eq!(strings(r#""hello""#), vec!["hello".to_string()]);
    }

    #[test]
    fn string_escapes_are_decoded_by_the_scanner() {
        assert_eq!(strings(r#""a\nb\tc""#), vec!["a\nb\tc".to_string()]);
        assert_eq!(strings(r#""\\ \" \' \0""#), vec!["\\ \" ' \0".to_string()]);
        assert_eq!(
            strings(r#""\x41\u{1F600}""#),
            vec!["A\u{1F600}".to_string()]
        );
    }

    #[test]
    fn braces_in_string_text() {
        // 3.4.2: `}` khong escape thi la ky tu thuong, `\{` viet ra `{`
        assert_eq!(strings(r#""a } b""#), vec!["a } b".to_string()]);
        assert_eq!(strings(r#""\{a\}""#), vec!["{a}".to_string()]);
        assert_eq!(body(r#""a } b""#), vec![StringStart, StringText, StringEnd]);
    }

    #[test]
    fn interpolation_reuses_the_ordinary_token_stream() {
        assert_eq!(
            body(r#""T\u{EA}n: {name}, tu\u{1ED5}i: {age}""#),
            vec![
                StringStart,
                StringText,
                InterpStart,
                Ident,
                InterpEnd,
                StringText,
                InterpStart,
                Ident,
                InterpEnd,
                StringEnd
            ]
        );
    }

    #[test]
    fn interpolations_hold_whole_expressions() {
        assert_eq!(
            body(r#""{a + b.c(1)}""#),
            vec![
                StringStart,
                InterpStart,
                Ident,
                Plus,
                Ident,
                Dot,
                Ident,
                LParen,
                IntLit,
                RParen,
                InterpEnd,
                StringEnd
            ]
        );
    }

    #[test]
    fn interpolations_nest() {
        assert_eq!(
            body(r#""{f("{x}")}""#),
            vec![
                StringStart,
                InterpStart,
                Ident,
                LParen,
                StringStart,
                InterpStart,
                Ident,
                InterpEnd,
                StringEnd,
                RParen,
                InterpEnd,
                StringEnd
            ]
        );
    }

    #[test]
    fn interpolation_nesting_is_bounded() {
        let mut source = String::new();
        for _ in 0..MAX_INTERPOLATION_DEPTH + 4 {
            source.push_str("\"{");
        }
        source.push('x');
        for _ in 0..MAX_INTERPOLATION_DEPTH + 4 {
            source.push_str("}\"");
        }
        assert_eq!(codes(&source), vec![ErrorCode::InterpolationTooDeep]);
    }

    #[test]
    fn broken_strings_are_reported_and_the_file_keeps_scanning() {
        assert_eq!(codes(r#""{}""#), vec![ErrorCode::EmptyInterpolation]);
        assert_eq!(codes("\"abc"), vec![ErrorCode::UnterminatedString]);
        assert_eq!(codes("\"abc\nlet a = 1"), vec![ErrorCode::NewlineInString]);
        assert_eq!(codes(r#""\q""#), vec![ErrorCode::UnknownEscape]);
        assert_eq!(codes(r#""\x80""#), vec![ErrorCode::AsciiEscapeOutOfRange]);

        // cai newline ket thuc literal van con do cho phan 8 dung
        let (tokens, _) = scan("\"abc\nlet a = 1");
        let scanned: Vec<TokenKind> = tokens.into_iter().map(|token| token.kind).collect();
        assert_eq!(
            scanned,
            vec![
                StringStart,
                StringText,
                StringEnd,
                Terminator,
                Let,
                Ident,
                Eq,
                IntLit,
                Terminator,
                Eof
            ]
        );
    }

    #[test]
    fn a_newline_inside_an_interpolation_is_an_error() {
        assert_eq!(codes("\"{a\nb}\""), vec![ErrorCode::NewlineInString]);
    }

    // ===== comments =====
    #[test]
    fn line_comments_are_whitespace_and_keep_their_newline() {
        assert_eq!(body("a // comment\nb"), vec![Ident, Terminator, Ident]);
        assert_eq!(body("// only a comment"), Vec::<TokenKind>::new());
    }

    #[test]
    fn block_comments_nest() {
        assert_eq!(body("a /* x /* y */ z */ b"), vec![Ident, Ident]);
        assert_eq!(
            codes("/* unterminated"),
            vec![ErrorCode::UnterminatedBlockComment]
        );
        assert_eq!(
            codes("/* outer /* inner */"),
            vec![ErrorCode::UnterminatedBlockComment]
        );
    }

    #[test]
    fn a_block_comment_spanning_a_newline_behaves_as_a_newline() {
        assert_eq!(body("a /* \n */ b"), vec![Ident, Terminator, Ident]);
        assert_eq!(body("a /* x */ b"), vec![Ident, Ident]);
    }

    // ===== terminator insertion =====
    #[test]
    fn a_newline_after_a_closer_ends_the_statement() {
        assert_eq!(
            body("let a = 1\nlet b = 2"),
            vec![Let, Ident, Eq, IntLit, Terminator, Let, Ident, Eq, IntLit]
        );
    }

    #[test]
    fn a_line_ending_in_an_operator_or_an_open_bracket_continues() {
        assert_eq!(
            body("let n = a +\n b"),
            vec![Let, Ident, Eq, Ident, Plus, Ident]
        );
        assert_eq!(
            body("f(\n a,\n b\n)"),
            vec![Ident, LParen, Ident, Comma, Ident, RParen]
        );
        assert_eq!(
            body("[\n1,\n2\n]"),
            vec![LBracket, IntLit, Comma, IntLit, RBracket]
        );
    }

    #[test]
    fn a_leading_operator_or_dot_continues_the_previous_line() {
        assert_eq!(
            body("let n = a\n + b"),
            vec![Let, Ident, Eq, Ident, Plus, Ident]
        );
        assert_eq!(
            body("let r = items\n .map(f)\n .filter(g)"),
            vec![
                Let, Ident, Eq, Ident, Dot, Ident, LParen, Ident, RParen, Dot, Ident, LParen,
                Ident, RParen
            ]
        );
    }

    #[test]
    fn else_and_catch_may_start_their_own_line() {
        assert_eq!(
            body("if x {\n}\nelse {\n}"),
            vec![If, Ident, LBrace, RBrace, Else, LBrace, RBrace]
        );
        assert_eq!(
            body("f()\ncatch {\nreturn\n}"),
            vec![Ident, LParen, RParen, Catch, LBrace, Return, RBrace]
        );
    }

    #[test]
    fn an_open_bracket_never_glues_onto_the_previous_line() {
        // 8.3: `(` va `[` khong nam trong tap nuot terminator, chinh cho nay
        // giet cai bay cham phay tu dong cua JavaScript
        assert_eq!(
            body("let a = b\n(c).d()"),
            vec![
                Let, Ident, Eq, Ident, Terminator, LParen, Ident, RParen, Dot, Ident, LParen,
                RParen
            ]
        );
        assert_eq!(
            body("let a = b\n[0]"),
            vec![Let, Ident, Eq, Ident, Terminator, LBracket, IntLit, RBracket]
        );
    }

    #[test]
    fn a_returned_value_must_begin_on_the_return_line() {
        assert_eq!(body("return\nx"), vec![Return, Terminator, Ident]);
        assert_eq!(body("return x"), vec![Return, Ident]);
    }

    #[test]
    fn braces_keep_newline_separation_so_struct_literals_need_no_commas() {
        assert_eq!(
            body("User {\n name: 1\n age: 2\n}"),
            vec![Ident, LBrace, Ident, Colon, IntLit, Terminator, Ident, Colon, IntLit, RBrace]
        );
    }

    #[test]
    fn an_explicit_semicolon_separates_statements_on_one_line() {
        assert_eq!(
            body("let a = 10; let b = 20"),
            vec![Let, Ident, Eq, IntLit, Semicolon, Let, Ident, Eq, IntLit]
        );
        assert_eq!(body(";;"), vec![Semicolon, Semicolon]);
    }

    #[test]
    fn blank_lines_never_produce_two_terminators() {
        assert_eq!(
            body("let a = 1\n\n\nlet b = 2"),
            vec![Let, Ident, Eq, IntLit, Terminator, Let, Ident, Eq, IntLit]
        );
    }

    #[test]
    fn end_of_file_terminates_a_statement_that_ends_in_a_closer() {
        assert_eq!(
            kinds("let a = 1"),
            vec![Let, Ident, Eq, IntLit, Terminator, Eof]
        );
        assert_eq!(kinds("let a ="), vec![Let, Ident, Eq, Eof]);
        assert_eq!(kinds(""), vec![Eof]);
    }

    #[test]
    fn a_terminator_before_a_closing_brace_is_elided() {
        assert_eq!(
            body("{ let a = 1 }"),
            vec![LBrace, Let, Ident, Eq, IntLit, RBrace]
        );
        assert_eq!(
            body("{\n let a = 1\n}"),
            vec![LBrace, Let, Ident, Eq, IntLit, RBrace]
        );
    }

    // ===== import paths =====
    #[test]
    fn a_backslash_is_a_token_only_inside_an_import_path() {
        assert_eq!(
            body(r"import net\http"),
            vec![Import, Ident, Backslash, Ident]
        );
        assert_eq!(
            body(r"import a\b\c as d"),
            vec![Import, Ident, Backslash, Ident, Backslash, Ident, As, Ident]
        );
        assert_eq!(
            codes(r"let a = b \ c"),
            vec![ErrorCode::BackslashOutsideImportPath]
        );
        assert_eq!(
            codes("import net\\http\nlet a = b \\ c"),
            vec![ErrorCode::BackslashOutsideImportPath]
        );
    }

    #[test]
    fn an_import_path_may_not_be_continued_on_the_next_line() {
        assert_eq!(
            codes("import net\\\n http"),
            vec![ErrorCode::MultilineImportPath]
        );
        assert_eq!(
            body("import io\nlet a = 1"),
            vec![Import, Ident, Terminator, Let, Ident, Eq, IntLit]
        );
    }

    // ===== operators =====
    #[test]
    fn operators_are_scanned_by_maximal_munch() {
        assert_eq!(body("x!=y"), vec![Ident, BangEq, Ident]);
        assert_eq!(body("a<<b"), vec![Ident, Shl, Ident]);
        assert_eq!(body("a<-b"), vec![Ident, Lt, Minus, Ident]);
        assert_eq!(body("a..=b"), vec![Ident, DotDotEq, Ident]);
        assert_eq!(body("a::b"), vec![Ident, ColonColon, Ident]);
        assert_eq!(body("a...b"), vec![Ident, Ellipsis, Ident]);
    }

    #[test]
    fn every_operator_in_the_precedence_table_scans() {
        assert_eq!(
            body("+ - * / % == != < > <= >= && || ! & | ^ << >> = += -= *= /= %= .. ..= . ? => :: ..."),
            vec![
                Plus, Minus, Star, Slash, Percent, EqEq, BangEq, Lt, Gt, LtEq, GtEq, AmpAmp,
                PipePipe, Bang, Amp, Pipe, Caret, Shl, Shr, Eq, PlusEq, MinusEq, StarEq, SlashEq,
                PercentEq, DotDot, DotDotEq, Dot, Question, FatArrow, ColonColon, Ellipsis
            ]
        );
    }

    #[test]
    fn all_punctuation_scans() {
        assert_eq!(
            body("( ) [ ] { } , : ;"),
            vec![LParen, RParen, LBracket, RBracket, LBrace, RBrace, Comma, Colon, Semicolon]
        );
    }

    #[test]
    fn there_is_no_optional_chaining_token() {
        // 4.3: `x?.y` la `(x?).y`, ba token cong voi reciever
        assert_eq!(body("x?.y"), vec![Ident, Question, Dot, Ident]);
        assert_eq!(body("x??"), vec![Ident, Question, Question]);
    }

    #[test]
    fn a_double_close_angle_reaches_the_parser_so_it_can_split_it() {
        // 9.2 can `>>` va `=` la hai token rieng o day
        assert_eq!(
            body("let b: Box<Box<int>>= v"),
            vec![Let, Ident, Colon, Ident, Lt, Ident, Lt, Ident, Shr, Eq, Ident]
        );
        assert_eq!(body("a >> b"), vec![Ident, Shr, Ident]);
    }

    #[test]
    fn operators_pump_does_not_have_get_their_own_message() {
        for source in [
            "a ~ b", "a ** b", "a ++ b", "a -- b", "a -> b", "a &= b", "a |= b", "a ^= b",
            "a <<= b", "#", "$",
        ] {
            assert_eq!(
                codes(source),
                vec![ErrorCode::OperatorNotInPump],
                "for {source:?}"
            );
        }
    }

    #[test]
    fn brackets_must_balance() {
        assert_eq!(codes("f(a"), vec![ErrorCode::UnclosedBracketAtEof]);
        assert_eq!(codes("a)"), vec![ErrorCode::UnmatchedClosingBracket]);
        assert!(codes("f([{ }])").is_empty());
    }

    // ===== source text and spans =====
    #[test]
    fn a_byte_order_mark_is_accepted_and_discarded() {
        assert_eq!(body("\u{feff}let a = 1"), vec![Let, Ident, Eq, IntLit]);
        let (tokens, _) = scan("\u{feff}let");
        assert_eq!((tokens[0].span.line, tokens[0].span.column), (1, 1));
    }

    #[test]
    fn crlf_is_one_line_terminator_and_a_lone_cr_is_an_error() {
        assert_eq!(
            body("let a = 1\r\nlet b = 2"),
            vec![Let, Ident, Eq, IntLit, Terminator, Let, Ident, Eq, IntLit]
        );
        assert_eq!(codes("a\rb"), vec![ErrorCode::LoneCarriageReturn]);
    }

    #[test]
    fn invisible_characters_are_rejected() {
        assert_eq!(codes("a\u{a0}b"), vec![ErrorCode::InvalidWhitespace]);
        assert_eq!(codes("a\u{2028}b"), vec![ErrorCode::InvalidWhitespace]);
        assert_eq!(codes("a\u{b}b"), vec![ErrorCode::InvalidWhitespace]);
        assert!(codes("a\tb").is_empty());
    }

    #[test]
    fn identifiers_are_ascii_only() {
        assert_eq!(
            codes("let caf\u{e9} = 1"),
            vec![ErrorCode::NonAsciiIdentifier]
        );
    }

    #[test]
    fn spans_carry_exact_byte_offsets_lines_and_columns() {
        let (tokens, codes) = scan("let a = 1\nlet bb = 22");
        assert!(codes.is_empty());

        let first = &tokens[0];
        assert_eq!((first.span.start, first.span.end), (0, 3));
        assert_eq!((first.span.line, first.span.column), (1, 1));

        let second_let = tokens
            .iter()
            .find(|token| token.kind == Let && token.span.line == 2)
            .expect("the second `let`");
        assert_eq!((second_let.span.start, second_let.span.end), (10, 13));
        assert_eq!((second_let.span.line, second_let.span.column), (2, 1));

        let last_int = tokens
            .iter()
            .rev()
            .find(|token| token.kind == IntLit)
            .expect("the last integer");
        assert_eq!((last_int.span.line, last_int.span.column), (2, 10));
    }

    #[test]
    fn columns_are_byte_counts_that_stay_correct_across_multi_byte_text() {
        // `let s = "T\u{EA}n"` - the `\u{EA}` occupies two bytes.
        let source = "let s = \"T\u{ea}n\"\nlet t = 1";
        let (tokens, codes) = scan(source);
        assert!(codes.is_empty());

        let string_end = tokens
            .iter()
            .find(|token| token.kind == StringEnd)
            .expect("the closing quote");
        assert_eq!(string_end.span.start, 13);
        assert_eq!((string_end.span.line, string_end.span.column), (1, 14));

        let second_line = tokens
            .iter()
            .find(|token| token.span.line == 2)
            .expect("a token on line 2");
        assert_eq!(second_line.span.column, 1);
    }

    #[test]
    fn every_token_carries_a_real_span() {
        let (tokens, _) = scan("fn main() {\n    print(\"hi {x}\")\n}");
        for token in &tokens {
            assert!(!token.span.is_synthetic(), "{token:?}");
            assert!(token.span.start <= token.span.end, "{token:?}");
            assert!(token.span.line >= 1 && token.span.column >= 1, "{token:?}");
        }
    }

    // ===== whole programs =====
    #[test]
    fn the_specification_example_scans_without_diagnostics() {
        let source = r#"
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
"#;
        assert!(codes(source).is_empty(), "{:?}", codes(source));
    }

    #[test]
    fn the_wider_feature_set_scans_without_diagnostics() {
        let source = r#"
// nhet moi thu grammar dinh nghia vao mot file cho de thu
/* including /* a nested */ block comment */
import net\http as web

pub const LIMITS: [int] = [1_000, 0xFF, 0o17, 0b1010]

enum Result<T, E> {
    Ok(T)
    Err(E)
}

interface Printable {
    fn print()
}

implements User: Printable

struct Box<T> {
    value: T
    callback: fn(int): int

    fn map(f: fn(T): T): Box<T> {
        return Box { value: f(this.value) }
    }
}

fn connect(host: string, port: int = 80, tags: ...string): string! {
    let ids: set<int> = set{}
    let users: [string: User] = {}
    let point: (int, int) = (10, 20)
    let n = point.0 + point.1
    let ok = a & b == c && !d || e ^ f << g >> h
    let r = 0..=10
    let maybe = find(1)?
    let data = read_file("x") catch { return "" }
    let add = fn(a: int, b: int): int { return a + b }

    match value {
        0 => print("zero")
        1 | 2 => print("small")
        Color.Red => { print("red") }
        _ if n > 0 => print("other")
        _ => print("none")
    }

    for i in 0..10 {
        if i == 3 { continue }
        if i == 7 { break }
        web.get("http://x/{i}?q={n}")
    }

    while n < 10 {
        n += 1
    }

    fail "unreachable"
}
"#;
        assert!(codes(source).is_empty(), "{:?}", codes(source));
    }

    #[test]
    fn scanning_never_stops_at_the_first_error() {
        let source = "let a = ~1\nlet b = 'ab'\nlet c = \"x";
        let reported = codes(source);
        assert_eq!(
            reported,
            vec![
                ErrorCode::OperatorNotInPump,
                ErrorCode::CharLiteralTooLong,
                ErrorCode::UnterminatedString,
            ]
        );
    }

    #[test]
    fn an_interpolation_left_open_at_end_of_file_is_reported_once_per_frame() {
        assert_eq!(
            codes("\"{a"),
            vec![
                ErrorCode::UnterminatedString,
                ErrorCode::UnclosedBracketAtEof,
            ]
        );
    }

    #[test]
    fn adversarial_input_terminates_and_still_produces_a_stream() {
        let sources = [
            "\"{\"{\"{",
            "'''''",
            "0x0x0x",
            "..........",
            "{{{{{{",
            "))))))",
            "0.0.0.0",
            "//",
            "/*/*/*",
            "\u{feff}",
            "\r\r\r",
            "\"\\",
            "1e+e+1",
        ];
        for source in sources {
            let (tokens, _) = scan(source);
            assert_eq!(
                tokens.last().map(|token| token.kind),
                Some(Eof),
                "{source:?}"
            );
        }
    }

    #[test]
    fn the_stream_always_ends_in_exactly_one_eof() {
        for source in ["", "let", "\"unterminated", "f(", "/* open"] {
            let (tokens, _) = scan(source);
            assert_eq!(
                tokens.last().map(|token| token.kind),
                Some(Eof),
                "{source:?}"
            );
            assert_eq!(
                tokens.iter().filter(|token| token.kind == Eof).count(),
                1,
                "{source:?}"
            );
        }
    }
}
