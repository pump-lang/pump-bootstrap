// compiler cua Pump.
//
//   .pump -> lexer -> parser -> resolve -> check -> lower -> IR
//                                                            |
//                                      +---------------------+
//                                      |
//                                   clif.rs
//                                    /    \
//                                jit.rs   object + link.rs -> .exe
//
// ai doc cay nay lan dau thi doc theo thu tu nay: grammar/pump.ebnf, roi
// token voi ast, roi types, roi abi doc kem docs/abi.md, cuoi cung la ir.

pub mod abi;
pub mod ast;
pub mod check;
pub mod clif;
pub mod errors;
pub mod ir;
pub mod jit;
pub mod lexer;
pub mod link;
pub mod lower;
pub mod parser;
pub mod resolve;
pub mod token;
pub mod types;
