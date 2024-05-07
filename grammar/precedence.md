# Pump 1.0 - Operator Precedence and Associativity

Normative companion to `pump.ebnf`. The layered expression rules in section 14
of the grammar are the machine-readable form of this table; if the two ever
disagree, the grammar wins and this file is the bug.

Level 1 binds tightest.

| # | Category | Operators | Associativity | Grammar rule |
|---|----------|-----------|---------------|--------------|
| 1 | Postfix | `.f` `.0` `(...)` `[...]` `?` `!` `::<T>` `{...}` (struct literal) | left | `PostfixExpression` |
| 2 | Unary prefix | `!` `-` | right | `UnaryExpression` |
| 3 | Multiplicative | `*` `/` `%` | left | `MultiplicativeExpression` |
| 4 | Additive | `+` `-` | left | `AdditiveExpression` |
| 5 | Shift | `<<` `>>` | left | `ShiftExpression` |
| 6 | Bitwise AND | `&` | left | `BitAndExpression` |
| 7 | Bitwise XOR | `^` | left | `BitXorExpression` |
| 8 | Bitwise OR | `\|` | left | `BitOrExpression` |
| 9 | Comparison | `==` `!=` `<` `>` `<=` `>=` | **non-associative** | `ComparisonExpression` |
| 10 | Logical AND | `&&` | left, short-circuit | `AndExpression` |
| 11 | Logical OR | `\|\|` | left, short-circuit | `OrExpression` |
| 12 | Range | `..` `..=` | **non-associative** | `RangeExpression` |
| 13 | Error fallback | `catch` | left | `CatchExpression` |

**Assignment is not on this table.** `=` `+=` `-=` `*=` `/=` `%=` are
*statements*, not operators (`AssignmentStatement`, grammar 13.1). They cannot
appear inside an expression, so they have no precedence and no associativity.
`a = b = c` does not parse, and `if a = b` does not parse.

---

## Why this order

**Bitwise binds tighter than comparison (levels 6-8 above level 9).**
C's ordering makes `a & b == c` mean `a & (b == c)`, which has produced bugs
for fifty years. Pump follows Rust and Go: `a & b == c` is `(a & b) == c`,
which is what it looks like.

**Shift sits between additive and bitwise AND (level 5).**
`a + b << c` is `(a + b) << c`. C would agree; the placement matters mainly
because it puts `<<` below `+`, so the common `base + offset << shift` reads
left to right.

**Comparison is non-associative (level 9).**
`a < b < c` is a *parse error*, not `(a < b) < c`. Two reasons:

1. It catches the classic mathematical-notation bug at parse time.
2. It closes the generic-call ambiguity for good. Because `<` can never chain,
   and because explicit type arguments in an expression are always written
   with turbofish (`f::<int>(x)`), there is no token sequence in expression
   position where the parser must guess whether `<` opens a type-argument
   list. Inside an expression, `<` is a comparison. Always.

The error message is fixed:

```
error: comparison operators cannot be chained
  help: use parentheses, or split with `&&`
```

**Range is non-associative and sits below `||` (level 12).**
`a..b..c` is a parse error. Placing `..` this low means `0..n-1` is
`0..(n - 1)`, which is what everyone means, and `0..items.length` is
`0..(items.length)`. Both endpoints are mandatory; `a..`, `..b` and `..` are
not expressions in 1.0.

**`catch` is the loosest expression form (level 13).**
`let x = a + b catch 0` is `(a + b) catch 0`. Putting `catch` anywhere tighter
would make `f(a) catch 0` bind the `catch` to `a` instead of to the call, which
is never what is wanted. It stays above assignment only because assignment is
not an expression at all.

**Postfix is one flat, tightest level (level 1) and applies left to right.**
There is no precedence *within* level 1 - the chain is just consumed in source
order. This is the rule that makes `?` and `!` behave:

```
user?.name      is  ((user)?).name
f()!.g()        is  ((f())!).g()
a[0]?.b!        is  (((a[0])?).b)!
box::<int>.get  is  ((box::<int>)).get
```

`user?.name` reads like optional chaining and means "propagate null, then take
`.name`", which is the same thing. There is deliberately no `?.` token.

**Unary binds looser than postfix (level 2 below level 1).**
`-x.y` is `-(x.y)`. `!user.active` is `!(user.active)`. `-a!` is `-(a!)`.
This is universal across languages and there is no reason to differ.

---
