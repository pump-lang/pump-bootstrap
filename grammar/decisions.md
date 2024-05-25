# Pump 1.0 - Grammar Decisions

Every decision that shapes `pump.ebnf`, with its reasoning and the
alternatives that were rejected. This document is meant to stand alone: the
four decisions the project owner already made are restated here in full, so a
reader never has to reconstruct them from a chat log.

Format for each entry: **Decision**, then **Why**, then **Rejected** where
there was a real fork in the road.

Cross-references from the grammar use these `D-n` numbers. They are stable;
new decisions get new numbers rather than renumbering.

---

## Part 0 - Decisions made by the project owner

These four were settled before this grammar was written. They are implemented
exactly as stated and are not relitigated here.

### P-1. Explicit type arguments use turbofish

**Decision.** Type arguments are **inferred by default**. Explicit
instantiation in an expression is written `zero::<int>()`, never
`zero<int>()`. Inside an expression, `<` is **always** a comparison operator
and never opens a type-argument list. Declarations and type positions keep
plain angle brackets, where no ambiguity exists: `fn first<T>(items: [T]): T?`,
`struct Box<T>`, `let b: Box<int>`, `set<int>`.

**Why.** `first<int>(items)` is genuinely indistinguishable from
`a < b > (c)` without unbounded lookahead or type information. Turbofish makes
the expression grammar LL(2) and keeps the parser a single pass.

The grammar reinforces this with a second, independent guard: comparison
operators are **non-associative** (D-10), so `a < b < c` is a parse error.
There is no token sequence in expression position where a parser could even be
tempted to guess.

Grammar: `PostfixOperator` turbofish alternative, NOTE 14.16.

### P-2. In expression position, `{}` is always a map

**Decision.** An empty or non-empty `{ ... }` in **expression** position is a
**map literal**. Sets are written `set{}`, `set{1, 2}`. Blocks exist only in
**statement** position, so all three never compete.

**Why.** The draft spec had `{}` meaning map, set, and block simultaneously.
Separating them by position (expression vs statement) plus one keyword (`set`)
removes the ambiguity with no lookahead at all.

Both spec lines parse unchanged:

```pump
let users: [string: User] = {}
let ids: set<int> = set{}
```

Grammar: NOTE 14.18, NOTE 13.0.1. Consequence recorded in NOTE 14.24: a
map literal's keys are ordinary expressions, so `{ name: "x" }` is a map keyed
by the *value* of `name`, not a struct literal with a forgotten type name.
That is the accepted cost.

### P-3. Bare struct literals are forbidden in control-flow headers

**Decision.** A bare struct literal may not appear in the header of `if`,
`while`, `for`, or `match`. Wrap it in parentheses:

```pump
if user == (User { name: "x" }) {
```

This is Go's rule. The parser must emit a **specific** diagnostic, not a
generic parse error:

```
error: struct literals are not allowed in the header of if / while / for / match
  help: wrap it in parentheses
        if user == (User { name: "x" }) {
```

**Why.** `if user == User { name: "x" } { ... }` has two readings and no local
information distinguishes them.

The grammar implements this as parser mode `ns` (grammar 9.1), entered for the
condition of `if`/`while`, the iterable of `for`, the scrutinee of `match`, a
match arm guard, and the operand of `catch`. In mode `ns` a `{` never begins a
struct or map literal. The mode is cleared inside `(`, `[`, and call argument
lists, and re-entered for a nested header.

`set{...}` is exempt: `set` is a keyword and cannot begin a block, so
`while set{1, 2}.has(x) { ... }` is unambiguous.

Extended to `catch` by D-17, for the identical reason.

### P-4. Concurrency is out of scope

**Decision.** `spawn`, channels, and everything concurrency-related are not in
Pump 1.0. `spawn` and `channel` are **reserved words** - using one is an error
reading "reserved for a future version of Pump", never a generic parse error.

**Why.** The draft spec itself classes these as runtime/stdlib API rather than
core language. Reserving the words now keeps a future addition source
compatible.

---

## Part 1 - Statement termination and line continuation

### D-1. Newline-driven terminator insertion, keyed on the last token

**Decision.** The scanner turns a newline into a `terminator` token when the
last significant token on the line is in the **closer set**:

```
identifier
_                                     (the wildcard token)
integer, float, char literal, closing " of a string
keywords:  true  false  null  this  return  break  continue
closers:   )  ]  }
postfix:   ?  !
```

Otherwise the newline is discarded.

**Why.** Exactly these tokens can end a complete expression, statement, type,
or block. Everything else - a binary operator, `,`, `:`, `=`, `=>`, `.`,
`..`, `::`, or an opening bracket - is by construction incomplete, so the line
must continue. This gives the brief's requirement, "a line ending in a binary
operator or an open bracket continues", for free, with no rule of its own and
no backslash continuation.

This is Go's automatic semicolon insertion with a Pump-shaped closer set. The
additions relative to Go are `?` and `!` (Pump's postfix operators) and `_`.

**Rejected.** Backslash line continuation - it collides with import paths
(D-16) and with the string escape character, and it is a second way to say
what the closer set already says.

Grammar: section 8.2.

### D-2. Suppression inside `(` and `[`; elision by lookahead

**Decision.** Two refinements on top of D-1.

**Bracket suppression.** While the innermost open bracket is `(`, `[`, or a
string interpolation `{`, no terminator is inserted at all. While it is `{`, or
there is no open bracket, insertion is active.

**Elision.** An inserted terminator is discarded when the next significant
token *cannot begin a statement*. That set is exactly:

```
else  catch
.  ,  )  ]  }  :  =>  ::
+  -  *  /  %
== != <  >  <= >=
&& || &  |  ^  << >>
.. ..=
=  += -= *= /= %=
```

and nothing else. `(`, `[`, `{`, `!`, `?`, and every other keyword are
deliberately **excluded**.

**Why.** Bracket suppression is what makes the spec's comma-free struct literal
work while call arguments still need commas. `{`-delimited constructs (blocks,
struct/enum/interface bodies, match bodies, struct/map/set literals) keep
newline separation; `(`/`[`-delimited ones (arguments, parameters, arrays,
tuples, type arguments) wrap freely and use commas. That is one rule, not two
special cases.

Elision buys three things people actually write:

```pump
if x {
}
else {                    // `else` on its own line
}

let r = items
    .map(f)               // leading-dot chains
    .filter(g)

let n = a
      + b                 // leading-operator arithmetic
```

The exclusions matter more than the inclusions. Excluding `(` and `[` kills
JavaScript's classic ASI hazard: these are two statements, not one call.

```pump
let a = b
(c).d()
```

Excluding `!` and `?` prevents a following `!cond` or a stray `?` from silently
gluing onto the previous line.

**Consequence users must be told.** `return`, `break` and `continue` are in the
closer set, so a returned value **must** start on the same line as `return`.
`return\n    x` is a bare return followed by a separate statement. This is
Go's rule and it is the price of having no semicolons.

Grammar: sections 8.1, 8.3, 8.4.

### D-3. `;` stays legal; a bare `;` is an empty statement

**Decision.** An explicit `;` is a terminator anywhere a terminator is allowed,
so `let a = 10; let b = 20` works as the spec shows. A `term` is **elidable**
before `}` and before EOF, so `{ let a = 1 }` is legal on one line. A `term`
on its own is an empty statement with no effect.

**Why.** The spec says both "`;` is not required" and "a statement is ended by
a newline or by `}`". Both are honoured literally.

Grammar: section 6.

---

## Part 2 - Statements versus expressions

### D-4. Assignment is a statement, not an expression

**Decision.** `=` `+=` `-=` `*=` `/=` `%=` are statement forms. They do not
appear in the precedence table, cannot occur inside an expression, and do not
chain. `if a = b` is a parse error. `a = b = c` is a parse error.

**Why.** The `if (a = b)` bug is one of the most-reported classes of C defect,
and the value of an assignment expression is used approximately never in the
kind of code Pump is for. Removing it also shrinks the expression grammar by a
level and makes the statement dispatcher simpler.

The parser still parses a full postfix expression on the left and *then*
validates it as an l-value, so the diagnostic is "cannot assign to a call"
rather than an unhelpful parse failure.

Grammar: 13.1, NOTE 13.1.2.

### D-5. An expression statement must contain a call

**Decision.** The only expressions allowed in statement position are those
whose postfix chain contains at least one call, optionally followed by `?`,
`!`, further member access, further calls, and `catch` clauses.

Legal: `print(x)`, `user.greet()`, `read_file(p)!`,
`load() catch { return }`, `items.map(f).filter(g)`.

Errors, each reading "this expression has no effect": `a + b`, `x`,
`user.name`.

**Why.** Two payoffs. First, it is the spec's own position - the spec says
explicitly that `a + b` on its own line does **not** become a return. Under
this rule it is not even a statement. Second, it makes D-2's elision set
provably safe: since no statement can begin with `+`, `-`, `*`, `&`, `.` or
`=`, treating a line that starts with one of those as a continuation can never
swallow a real statement.

Grammar: 13.2.

---

## Part 3 - Numeric and primitive semantics

### D-6. `int` and `uint` are 64-bit; overflow wraps

**Decision.**

- `int` is signed 64-bit two's complement.
- `uint` is unsigned 64-bit.
- `float` is IEEE-754 binary64.
- There are no sized variants (`i8`, `u32`, ...) in 1.0. Those names are
  reserved.
- Overflow on `+`, `-`, `*`, `<<` **wraps**, two's complement, **identically in
  debug and release builds**. Checked and saturating variants live in the
  stdlib (`math.add_checked(a, b): int?`, and so on).
- Overflow in a **compile-time constant expression** is a compile **error**,
  not a wrap.

**Why.** 64-bit is the only width that does not need thinking about on the
target platform, and a single width keeps the type system and the numeric
tower small.

Wrapping is chosen over trapping for one dominant reason: **no
debug/release divergence**. Rust's decision to trap in debug and wrap in
release means a program's behaviour depends on its build profile, which is
the single most surprising thing about Rust arithmetic. A wrapping default is
deterministic, has no hidden branch, matches what the hardware and Cranelift
do natively, and makes the checked APIs an explicit, visible opt-in - which is
exactly Pump's "explicit behavior" philosophy.

Constant expressions are the exception because there is no runtime cost to
diagnosing them and a wrapped constant is always a typo.

**Rejected.** Trapping on overflow everywhere (a branch on every arithmetic
op, and a panic in code that legitimately wants modular arithmetic).
Arbitrary-precision integers (a GC allocation on every add; wrong for a
compiled systems-adjacent language).

### D-7. Division, shifts, and the absence of implicit conversion

**Decision.**

- Integer `/` and `%` by zero **panic**. It is not undefined and not wrapped.
- `%` takes the sign of the **dividend** (truncated division, as in C, Rust,
  Go).
- `int.min / -1` **wraps** to `int.min`, consistent with D-6, rather than
  panicking. This is stated explicitly because it is exactly the case that
  becomes a bug when it is left unstated.
- `>>` is **arithmetic** on `int` (sign-extending) and **logical** on `uint`.
- A shift count at or above the type's width yields `0`, or `-1` for a
  negative `int >>`. A negative shift count **panics**. Shift counts are
  **not** masked.
- **There are no implicit numeric conversions of any kind**, in either
  direction, `int` to `float` included. `int + uint` is a type error.
  Conversions are calls on the type name: `int(x)`, `uint(x)`, `float(x)`,
  `char(x)`, `string(x)`.
- Untyped literals adapt to context (grammar 3.5). An integer literal defaults
  to `int`; a float literal defaults to `float` and can never adopt `int`.
  A literal that does not fit its adopted type is a compile error.

**Why.** Masked shifts (`b & 63`) are an x86 artefact that produces
inexplicable results on any other target; defining large shifts as 0 is
portable and matches Go. No-implicit-conversion is the one rule that
eliminates the entire class of silent-precision-loss and
signed/unsigned-comparison bugs, and the cost - writing `float(n)` - is one
call at a boundary.

`-9223372036854775808` is handled by a narrow rule: an integer literal
directly under unary `-` may be as large as 2^63 (grammar NOTE 3.5.3).
Otherwise this famous constant would be unwritable.

### D-8. `char` is a Unicode scalar value; `string` is immutable UTF-8

**Decision.**

- `char` is a **Unicode scalar value**, 32 bits: `0..0x10FFFF` excluding the
  surrogate range `0xD800..0xDFFF`. Comparison is by code point. Arithmetic on
  `char` is not allowed; use `uint(c)`.
- `string` is **immutable** and **always valid UTF-8**. There is no way to
  construct an invalid one, which is why `\xHH` is capped at `0x7F` (it is an
  ASCII escape, not a byte escape).
- `s.length` is a **byte count**.
- **`string` is not indexable with `[]`.** `s[i]` is a compile error. Use
  `s.chars()`, `s.bytes()`, `s.byte_at(i)`, `s.slice(a, b)`.
- `for c in s` yields `char`.
- `+` concatenates. `==` and `<` compare byte-wise, which for UTF-8 is also
  code-point order.

**Why.** Scalar-value `char` is the only definition under which `char` is
closed under the operations people expect and under which a `string` is a
sequence of `char`.

Refusing `s[i]` is the load-bearing decision here. Every language that allows
it has to pick between O(1)-but-wrong (bytes, so `s[0]` of a non-ASCII string
is half a character) and correct-but-O(n) (scalars, so a loop is quadratic).
Both are traps. Naming the operation you want - `byte_at` or `chars()` - makes
the cost visible and the result unambiguous. `length` in bytes follows: it is
the O(1) answer, and it is the one an allocator or an I/O call needs.

**Rejected.** UTF-16 `char` (surrogate pairs leak into user code); grapheme
clusters as the unit (needs a Unicode table in the core language and a
versioned one at that).

---

## Part 4 - Operators

### D-9. Operators Pump 1.0 does not have, and their diagnostics

**Decision.** The following sequences are **not** tokens. Each is a lexical
error with a **specific** message rather than "unexpected character":

| Sequence | Message |
|---|---|
| `~` | `Pump 1.0 has no bitwise NOT; use x ^ -1` |
| `&=` `\|=` `^=` `<<=` `>>=` | `Pump 1.0 has only = += -= *= /= %=` |
| `->` | `Pump writes return types with ':', not '->'` |
| `**` | `Pump has no exponent operator; use math.pow` |
| `++` `--` | `Pump has no increment operator; use += 1` |
| `@` `#` `$` | no attribute, directive or sigil in Pump 1.0 |

`?.`, `??` and `?:` are likewise not tokens; `x?.y` already scans as `(x?).y`,
which is what it should mean.

**Why.** The spec's operator list has `& | ^ << >>` but no complement, and six
assignment operators but no bitwise-assign. Those are real gaps. But *closing*
them is an additive, non-breaking change the owner can make in 1.1, whereas
inventing operators now risks diverging from the owner's intent for the sake
of completeness. So: keep the operator set exactly as specified, and spend the
effort on error messages instead. A user who types `~x` gets told what to write
in one line, which is most of the value of having the operator, at none of the
risk.

**Recommended for 1.1**, flagged for owner sign-off: add unary `~`, and add
`&= |= ^= <<= >>=`. Both are pure additions.

### D-10. Precedence table; comparison and range are non-associative

**Decision.** Thirteen levels, given in full in `precedence.md`. The two
structural choices:

- **Bitwise `&`, `^`, `|` bind tighter than comparison.** `a & b == c` is
  `(a & b) == c`, not C's `a & (b == c)`.
- **Comparison and range are non-associative.** `a < b < c` and `a..b..c` are
  parse errors, not silent misreadings.

**Why.** C's bitwise precedence is a fifty-year-old bug that every modern
language has fixed; Pump follows Rust and Go.

Non-associative comparison does double duty: it catches the
mathematical-notation bug, and it is the second lock on P-1. With turbofish
required for explicit type arguments *and* `<` unable to chain, there is no
token sequence in expression position where the parser could be tempted to
treat `<` as a bracket.

Full table, worked examples, and the `>` splitting rule: `precedence.md`.

### D-11. Block comments nest

**Decision.** `/* a /* b */ c */` is one comment. A `//` comment does not
consume its trailing newline - that newline is still tested by D-1.

**Why.** Non-nesting block comments make "comment out this region" fail
silently whenever the region contains a comment. The scanner cost is one
counter.

---

## Part 5 - The overloaded punctuation

### D-12. `set` is a reserved keyword

**Decision.** `set` is one of the 27 keywords. It cannot be used as an
identifier.

**Why.** `set` introduces both a type constructor (`set<int>`) and a literal
(`set{1, 2}`). A contextual keyword - "keyword only when followed by `{` or in
type position" - would work, and was checked to be unambiguous. It was
rejected anyway: contextual keywords are the single largest source of parser
bugs and of confusing error messages, and Pump's stated philosophy is "simple
syntax, fast compilation". Reserving one common noun is a small, visible cost;
a contextual rule is an invisible one that keeps costing.

Note the asymmetry with the type names: `int`, `uint`, `float`, `bool`, `char`,
`string`, `void` and `Error` are **predeclared identifiers**, not keywords -
but they are **non-shadowable**, so declaring `let int = 3` is an error. `set`
is a keyword only because it also introduces a literal.

### D-13. Postfix `?` propagates null; it does not force-unwrap

**Decision.** `x?` on a value of type `T?` yields `T`. If the value is `null`,
it **returns `null` immediately from the enclosing function**, which must have
an optional return type. It is a compile error otherwise.

Force-unwrap and defaulting live in the stdlib: `x.expect("msg")` panics with
a message, `x.or(default)` substitutes.

`T?` in type position is the optional-type suffix. The two roles never compete
because types are parsed by a separate routine entered only from known type
positions (grammar 9.4).

**Why.** The spec flags `?` as needing disambiguation but does not say what
postfix `?` *means*. Two readings were available: force-unwrap-or-panic
(Kotlin `!!`, Swift `!`) or propagate (Rust `?`).

Propagate wins on symmetry. Pump already has `x!` meaning "propagate the
error"; making `x?` mean "propagate the null" gives one rule for both, with the
enclosing function's return type as the target in both cases. Force-unwrap
would make the two postfix operators mean opposite things - one propagates, one
panics - for no reason a user could remember.

It also fits "explicit behavior": a hidden panic is the least explicit thing a
one-character operator can do.

A pleasant consequence: because postfix operators chain left to right,
`user?.name` parses as `((user)?).name`. It *reads* like optional chaining and
*behaves* like propagation, which is the same thing at the use site. There is
deliberately no `?.` token.

**This is the decision most worth the owner's review**, since the spec's single
example (`let actual = user?`) is consistent with either reading.

### D-14. `T!` may appear only as a function return type

**Decision.** The `!` type suffix is legal **only** on a function's return
type, including the return type inside a function type. `let x: string!` is an
error:

```
error: `!` may only be used on a function's return type
  help: handle the error at the call site with `!` or `catch`
```

A value of a failable type must be consumed immediately by postfix `!` or by
`catch`. `let x = read_file(p)` where `read_file` returns `string!` is an error
reading "unhandled error; use `!` or `catch`".

**Why.** This is the whole point of the spec's line "the goal is explicit error
handling without turning into Rust's `Result<T, E>` everywhere". If `T!` were a
first-class type it would appear in struct fields, in collections, in generic
parameters, and Pump would have re-derived `Result` with worse syntax. Confining
it to the return type makes failability a property of *calling*, not of
*values*, and it means the checker never has to reason about a failable type in
an arbitrary position.

Suffix combinations: `T?!` is "error of optional", `T!?` is "optional of
error"; both parse, applied left to right. `T??` and `T!!` parse and are
rejected - optionals and error types do not nest.

`void` exists as a return type so that a failable function returning nothing is
`fn f(): void!`. A bare `!` return type is a syntax error with a hint pointing
at `void!` - one spelling, not two.

### D-15. `fail` - the error constructor the spec is missing

**Decision.** Add the statement `fail <expr>`. It is legal only inside a
function whose return type carries `!`, and it returns the error case. The
operand must conform to the builtin `Error` interface; `string` conforms, so
`fail "not found"` works.

```pump
fn read_file(path: string): string! {
    if !exists(path) {
        fail IoError { path: path }
    }
    return contents
}
```

`fail` is a keyword, and it is the **only** way to produce the error case.

**FLAGGED AS AN ADDITION BEYOND THE DRAFT SPEC.** The spec declares `T!`, shows
propagation with `!` and handling with `catch`, but never shows how a function
*produces* an error. Without something like `fail`, the feature is unusable -
no program can ever reach the error path.

**Why this shape.** The alternatives were worse. Overloading `return` so that
"an expression whose type conforms to `Error` means the error case" is
ambiguous the moment `T` itself conforms to `Error`, and it makes the meaning
of a `return` depend on type inference. Wrapper functions (`return err(e)` /
`return ok(v)`) re-introduce the `Result` shape that D-14 exists to avoid.
`throw` carries exception baggage Pump explicitly does not want. `fail` is one
keyword, reads as prose, is perfectly symmetric with `return`, and is
unambiguous with zero lookahead.
