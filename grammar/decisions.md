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
