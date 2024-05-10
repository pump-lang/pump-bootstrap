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
