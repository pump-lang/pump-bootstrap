# The Pump ABI

**Status: normative.** This document and `src/abi.rs` are one specification in
two forms. The prose here is authoritative for humans; the constants in
`src/abi.rs` are authoritative for code, and they are checked against this
document by the unit tests at the bottom of that file.

Code generation (`src/clif.rs`) and the garbage collector (`runtime/src/gc.rs`)
implement the two halves of this contract and never see each other's source. A
disagreement between them is a segfault whose cause is unattributable. **Do not
change one side of anything below without changing the other.**

Target: `x86_64-pc-windows-msvc`. Little-endian, 64-bit pointers, `usize` is 8
bytes. Every compiled Pump function and every runtime entry point uses the
platform C calling convention (Cranelift `CallConv::triple_default`, Rust
`extern "C"`).

---

## 1. The object header

**Every heap object begins with the same 16-byte header.** No exceptions: heap
allocated objects, static string literals, and static payload-free enum
singletons all carry it.

```
offset  size  field     type   description
-----------------------------------------------------------------------------
   +0      4  type_id   u32    index into the global type descriptor table
   +4      4  flags     u32    bit 0 MARK, bit 1 IMMORTAL, bits 2..31 must be 0
   +8      8  size      u64    total object size in bytes, header included,
                               always a multiple of 16
  +16      -  payload          the object body starts here
-----------------------------------------------------------------------------
total header size: 16 bytes
```

* **`type_id`** indexes `pump_type_table` (section 8). Ids 0 through 15 are
  reserved (section 3); compiler-emitted descriptors start at 16.
* **`flags`**
  * `FLAG_MARK = 0x1` - the collector reached this object in the current mark
    phase. The collector clears it during sweep. Generated code must never
    write it.
  * `FLAG_IMMORTAL = 0x2` - the object lives in static data and is never swept
    and never freed. The collector still traverses it, so that references it
    holds keep their targets alive. String literals and payload-free enum
    singletons carry it.
  * All other bits are reserved and must be zero.
* **`size`** is the *total* size, header included, rounded up to 16. The
  collector uses it to step through a heap page and to compute a free block's
  size, so it must be exact for every object including variable-length ones
  (strings, closures, buffers).

**Alignment.** Every object starts at a 16-byte boundary. Because the header is
exactly 16 bytes, the payload is 16-byte aligned too, so an `f64` field is
always naturally aligned and never straddles a cache line.

A pointer to a Pump object always points at the header, i.e. at byte 0, never
at the payload.

---

## 2. Value representation

Pump's machine types are exactly five:

| Machine type | Width | Used for |
|---|---|---|
| `i8`  | 1 | `bool`, and every predicate the backend computes; 0 or 1 only |
| `i32` | 4 | `char`, a Unicode scalar value |
| `i64` | 8 | `int`, `uint`, and every collection slot |
| `f64` | 8 | `float` |
| `ptr` | 8 | every reference |

**Everything that is not one of `bool`, `int`, `uint`, `float`, `char` is a
pointer to a heap object.** That includes `string`, `[T]`, `[K: V]`, `set<T>`,
tuples, structs, enums, closures, interface values, and every optional.

Tuples are heap objects even though the language treats them as values. This is
sound because a tuple element is not assignable: `LValue` (grammar 13.1) admits
`.identifier` and `[expr]` but not `.0`, so `t.0 = x` does not parse and a
tuple's contents can never be observed to change after construction.

### 2.1 Optionals

**`T?` is always a pointer, and `null` is always the null pointer (0).**

* When `T` is a reference type, `T?` is that same pointer, with `null`
  represented by 0. No box, no cost.
* When `T` is `bool`, `int`, `uint`, `float` or `char`, `T?` is a pointer to a
  **box** (section 4.7) holding the widened value, with `null` represented by 0.

One rule, one comparison: a null test on any optional is `ptr == 0`.

### 2.2 Collection slots

**Every element slot in an array, and every key and value slot in a map or a
set, is exactly 8 bytes wide**, whatever the element type. This is what lets one
runtime implementation serve every instantiation.

Widening rules:

| Element type | Stored as |
|---|---|
| `bool`  | 0 or 1 in the low byte, other bytes zero |
| `char`  | the scalar value zero-extended to 64 bits |
| `int`   | the two's-complement value |
| `uint`  | the value |
| `float` | the IEEE-754 binary64 **bit pattern**, bitcast, not converted |
| any reference | the pointer |

Generated code that reads a slot into an `f64` must bitcast, not convert.

### 2.3 Failable calls

There is no multi-value return anywhere in the Pump ABI, because the Windows
x64 C calling convention has no portable form for one.

A function whose Pump return type is `T!` compiles to a function returning
`T`'s representation and nothing else. Failure travels through one runtime
global:

```
pump_error_slot : void*     // a data symbol exported by the runtime
```

* It is initialised to null, and is always a GC root.
* `fail e` compiles to: build the `Error` interface value for `e`, call
  `pump_error_set` with it, then return the zero value of the function's return
  representation (0 for `i8`/`i32`/`i64`/`ptr`, +0.0 for `f64`; for a `void!`
  function, just return).
* Immediately after **every** call to a failable function, generated code tests
  `pump_error_slot != null`.
  * Postfix `!` (propagate): on non-null, return the zero value immediately,
    leaving the slot set.
  * `catch` (handle): on non-null, call `pump_error_take`, which returns the
    error and clears the slot, then run the handler.
* The slot is null whenever no error is in flight. That invariant holds because
  grammar D-14 requires every failable call to be consumed immediately by `!`
  or by `catch`, so no failure can escape unhandled and unpropagated.

The backend may read and write `pump_error_slot` directly instead of calling
`pump_error_pending`, `pump_error_take` and `pump_error_set`. The two are
defined to be equivalent; the direct form is one load.

---

## 3. Reserved type ids

| Id | Name | Meaning |
|---|---|---|
| 0 | `TYPE_ID_INVALID` | never a live object; a freed cell carries it |
| 1 | `TYPE_ID_BUFFER` | a raw byte buffer, never traced by its own descriptor |
| 2 | `TYPE_ID_STRING` | a `string` |
| 3 | `TYPE_ID_BOX_SCALAR` | a box holding one non-pointer slot |
| 4 | `TYPE_ID_BOX_REF` | a box holding one pointer slot |
| 5 | `TYPE_ID_INTERFACE` | an interface value |
| 6-15 | - | reserved for future builtin shapes; descriptors must be present and inert |
| 16+ | `FIRST_USER_TYPE_ID` | compiler-emitted descriptors |

The compiler emits one descriptor per distinct struct, per distinct enum, and
per distinct instantiation of `[T]`, `[K: V]`, `set<T>`, tuple and closure.

---

## 4. Object layouts, byte for byte

All offsets are from the start of the object, header included. `H` denotes
`HEADER_SIZE` = 16.

### 4.1 String

Immutable, always valid UTF-8, bytes inline.

```
   +0   16  header                     type_id = TYPE_ID_STRING (2)
  +16    8  length     u64             byte count of the contents
  +24    8  hash       u64             cached FNV-1a 64, or 0 for "not computed"
  +32    N  bytes      u8[length]      the UTF-8 contents
+32+N    1  nul        u8              always 0, for cheap C interop
       ...  padding                    zero, up to the aligned size
```

Total size = `align16(32 + length + 1)`.

* `length` is a **byte** count, which is what `s.length` returns (grammar D-8).
* `hash` is computed lazily by `pump_string_hash`. A hash that comes out 0 is
  stored as 1, so 0 unambiguously means "not computed yet".
* The trailing NUL is not part of the string and is not counted in `length`.
* The collector never traces a string.
