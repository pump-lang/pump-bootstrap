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
