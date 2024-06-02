// cai nay scanner. chu vao,
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
// heap chu khong ...

#![allow(dead_code)]

use crate::errors::{CompileError, Diagnostics, ErrorCode};
use crate::token::{FileId, Span, Token, TokenKind, TokenValue};

// 128 la bua, gap bug thi tang len. Spec 3.4.4 chi doi it nhat 32.
const MAX_INTERPOLATION_DEPTH: usize = 128;

// dat tam vao cho literal nao giai ma khong ra, de day token con nguyen
// hinh dang va parser con di tiep qua cho bao loi duoc.
