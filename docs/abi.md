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

### 4.2 Array `[T]`

A fixed header pointing at a separately allocated, growable element buffer, so
that pushing does not move the array object.

```
   +0   16  header                     type_id = a per-instantiation descriptor
  +16    8  length     u64             number of live elements
  +24    8  capacity   u64             element slots the buffer can hold
  +32    8  data       ptr             the element buffer object, or null
  +40    8  modcount   u64             bumped by every structural change
```

Total size = 48, always.

* The element buffer is a `TYPE_ID_BUFFER` object. Element `i` lives at
  `data + 16 + i*8` - that is, at offset 16 **inside the buffer object**, past
  the buffer's own header.
* `capacity == 0` implies `data == null`.
* Growth doubles capacity, minimum 4.
* `modcount` starts at 0 and increments on push, pop, insert, remove, clear and
  any capacity change. A `for` loop over the array snapshots it and re-checks
  each iteration; a mismatch calls `pump_panic_concurrent_modification`
  (grammar 13.3.8).

### 4.3 Map `[K: V]`

Insertion-ordered (grammar D-26), Python-dict shaped: a dense entry buffer in
insertion order, plus an open-addressed index buffer of `i64` slots.

```
   +0   16  header                     type_id = a per-instantiation descriptor
  +16    8  length          u64        live entry count
  +24    8  entries         ptr        entry buffer object, or null
  +32    8  entry_capacity  u64        entry slots the buffer can hold
  +40    8  entry_used      u64        entry slots consumed, tombstones included
  +48    8  index           ptr        index buffer object, or null
  +56    8  index_capacity  u64        index slots, always a power of two
  +64    8  modcount        u64
  +72    4  key_kind        u32        KEY_KIND_*
  +76    4  slot_flags      u32        bit 0 value is a pointer, bit 1 key is
```

Total size = 80, always.

Entry buffer, stride 24, entry `i` at `entries + 16 + i*24`:

```
   +0    8  hash    u64     0 means empty or tombstoned
   +8    8  key     u64     widened per section 2.2
  +16    8  value   u64     widened per section 2.2
```

Index buffer, stride 8, an `i64` per slot at `index + 16 + i*8`:

| Value | Meaning |
|---|---|
| `-1` (`INDEX_EMPTY`) | no entry |
| `-2` (`INDEX_TOMBSTONE`) | an entry was here and was removed |
| `>= 0` | the entry's position in the entry buffer |

* **Iteration order is insertion order**, guaranteed and reproducible: walk
  `0..entry_used`, skipping entries whose `hash` is 0.
* A stored hash of 0 is impossible for a live entry; a computed hash of 0 is
  stored as 1.
* `key_kind` selects hashing and equality:

  | Value | Name | Behaviour |
  |---|---|---|
  | 0 | `KEY_KIND_SCALAR` | bitwise on the 8-byte slot; `int`, `uint`, `char`, `bool`, payload-free enums |
  | 1 | `KEY_KIND_STRING` | FNV-1a 64 over the UTF-8 bytes; equality by content |
  | 2 | `KEY_KIND_REFERENCE` | hash and compare the pointer itself |
  | 3 | `KEY_KIND_TUPLE` | structural; reserved, not produced in 1.0 |

* **`key_kind` is derived by the runtime, not supplied by the compiler.** The
  type descriptor carries no key-kind field - section 8's layout is fixed -
  and `DESC_FLAG_KEY_IS_REF` alone cannot tell a `string` key from any other
  reference key. So the runtime decides per operation:

  | Condition | Kind |
  |---|---|
  | `DESC_FLAG_KEY_IS_REF` clear | `KEY_KIND_SCALAR` |
  | key is a non-null object whose own `type_id` is `TYPE_ID_STRING` | `KEY_KIND_STRING` |
  | otherwise | `KEY_KIND_REFERENCE` |

  This is exact for every key Pump 1.0 can produce: pointer identity is also
  the correct rule for a payload-free enum key, whose variants are singletons.
  The `key_kind` word in the object is a cache of that decision, written at
  construction and corrected on the first insertion, so a debugger reading the
  object sees the truth. Nothing in generated code reads or writes it.

* `float` is never a key (grammar 17, D-26).

### 4.4 Set `set<T>`

**A set object has the identical 80-byte layout as a map**, with the entry
buffer's value column present but unused. One runtime implementation serves
both; only the descriptor kind differs. The 8 bytes per element that this
wastes buys a single hash table implementation, which is the right trade for
1.0.

### 4.5 Tuple and struct

Identical layout. Fields start at `+16` and follow in **declaration order**,
each at its natural alignment. **Fields are never reordered**, so a debugger's
view of a Pump struct matches its source.

Example - `struct User { flag: bool, initial: char, age: int, name: string }`:

```
   +0   16  header
  +16    1  flag       i8
  +17    3  padding
  +20    4  initial    i32
  +24    8  age        i64
  +32    8  name       ptr
```

Total size = `align16(40)` = 48. The descriptor's `ref_offsets` is `[32]`.

A struct with no fields is 16 bytes: a bare header.

### 4.6 Enum

A tag, then that variant's payload laid out exactly as struct fields are.

```
   +0   16  header
  +16    4  tag        u32        the variant's declaration index
  +20    4  padding    must be 0
  +24    -  payload               variant payload fields, struct-style
```

* **The tag is the variant's index in declaration order** (grammar 12.3). The
  checker assigns it, the backend emits it, the collector reads it.
* Instances are sized per variant, not padded to the largest one: an instance
  occupies only what its own variant needs. The collector reads the tag and
  uses that variant's pointer map.
* A payload-free variant produces an object of exactly 32 bytes
  (`align16(24)`).
* **Payload-free variants are static singletons.** For each payload-free
  variant the backend emits one read-only object with `FLAG_IMMORTAL` set and
  the correct tag, and every construction of that variant yields its address.
  `Color.Red` therefore allocates nothing, and `==` on payload-free enums is
  pointer equality.

### 4.7 Box

One 8-byte slot. Used for an optional primitive (section 2.1) and for a
captured binding (section 4.8).

```
   +0   16  header      type_id = TYPE_ID_BOX_SCALAR (3) or TYPE_ID_BOX_REF (4)
  +16    8  value       widened per section 2.2
```

Total size = `align16(24)` = 32.

### 4.8 Closure

```
   +0   16  header                     type_id = a per-instantiation descriptor
  +16    8  code            ptr        address of the compiled body
  +24    8  capture_count   u64
  +32    -  captures        ptr[n]     one pointer per captured binding
```

Total size = `align16(32 + 8*n)`.

* **`code` is a raw code pointer, not a GC object. The collector never follows
  it.**
* A closure captures **bindings, not values** (grammar D-30), so each capture
  slot points at a **box** (section 4.7), shared with every other closure and
  with the enclosing frame that captured the same binding. Assigning through
  one closure is visible through another.
* **Calling convention:** `code(closure_ptr, arg0, arg1, ...)`. The closure
  object is always physical parameter 0; the body loads its captures from it.
  A value of function type is always this pair, so a plain named function
  passed as a value is wrapped in a closure with zero captures.

### 4.9 Interface value

An interface value is a **boxed fat pointer**: one heap object holding the
itable and the data pointer.

```
   +0   16  header                     type_id = TYPE_ID_INTERFACE (5)
  +16    8  itable      ptr            static itable; NOT a GC object
  +24    8  data        ptr            the underlying object
```

Total size = 32.

Boxing rather than passing two machine words keeps every Pump value exactly one
machine word, which keeps the IR, the calling convention and the collector
uniform. The cost is one allocation per conversion to an interface type;
conversions are rarer than calls, and calls stay at two loads.

* When the concrete type is a primitive, `data` points at a **box**
  (section 4.7).
* The collector traces `data` and never traces `itable`.

### 4.10 Buffer

```
   +0   16  header      type_id = TYPE_ID_BUFFER (1)
  +16    -  payload     opaque bytes
```

A buffer's own descriptor traces **nothing**. Its contents are traced by the
object that owns it: an array traces its element buffer, a map or a set traces
its entry buffer. A buffer is therefore only ever *marked* through its owner or
by a conservative stack hit, never traced through its own descriptor. This is
why one buffer descriptor suffices for every element type.

---

## 5. Itables and dynamic dispatch

An itable is **static read-only data**, not a GC object, and has no header.

```
   +0    8  interface_id     u64    the interface's DefId
   +8    8  concrete_type_id u64    the concrete type's runtime type id
  +16    8  method_count     u64
  +24    8  method[0]        ptr
  +32    8  method[1]        ptr
   ...
```

Total size = `24 + 8 * method_count`.

**Slot numbering is the interface's method declaration order** (grammar 12.4).
Method `i` in the interface body occupies slot `i`. This ordering is the entire
dispatch contract: the checker assigns it, the backend emits itables in it, and
every dynamic call indexes by it.

Symbol name: `pumpvt$<interface module>.<Interface>$<concrete module>.<Concrete>`.

### Dispatch sequence

Calling interface slot `k` on an interface value `v` with arguments
`a1..an` is exactly:

```
itable = load ptr [v + 16]                 ; interface::ITABLE_OFFSET
data   = load ptr [v + 24]                 ; interface::DATA_OFFSET
fn     = load ptr [itable + 24 + 8*k]      ; itable_method_offset(k)
result = call_indirect fn(data, a1, ..., an)
```

The physical signature of the target is `(ptr, <arg types>) -> <ret>`: the
receiver is always physical parameter 0.

Because conformance is structural (grammar D-31) and parameter names are not
part of the match, a method reached through an interface can never be called
with named arguments, so the itable never needs to carry parameter metadata.

---

## 6. Function calling convention

### 6.1 Free functions

Physical parameters are the declared parameters in order, each in its machine
representation. Defaults are materialised at the **call site**: the caller
evaluates the constant and passes it. A variadic parameter `...T` is one
physical parameter of type `ptr`, holding a freshly built `[T]`; the caller
builds it.

### 6.2 Methods

A method's compiled function takes the receiver as **physical parameter 0**,
typed `ptr`, followed by the declared parameters. `this` inside the body is
that parameter.

### 6.3 Closures

`code(closure_ptr, arg0, ...)` - see section 4.8.

### 6.4 Symbol names

```
pump$<module>$<owner>$<name>$<type arguments>
```

Always five `$`-separated fields, so the name parses back unambiguously. Pump
identifiers are ASCII letters, digits and `_` only (grammar 2.2), so `$` can
never occur inside a field.

* `<module>` - module path joined with `.`, e.g. `net.http`
* `<owner>` - the receiver type's name for a method, empty for a free function
* `<name>` - the function name
* `<type arguments>` - the concatenated type encodings of a monomorphised
  instantiation, empty otherwise

Examples:

| Declaration | Symbol |
|---|---|
| `fn main()` in module `main` | `pump$main$$main$` |
| `fn greet()` on `User` in module `app` | `pump$app$User$greet$` |
| `fn first<T>` instantiated at `[int]` in `util` | `pump$util$$first$Ai` |

Type encoding (prefix-free, so a concatenation parses without separators):

| Type | Encoding |
|---|---|
| `bool` `int` `uint` `float` `char` `string` `void` | `b` `i` `u` `f` `c` `s` `v` |
| `[T]` | `A` then `T` |
| `[K: V]` | `M` then `K` then `V` |
| `set<T>` | `S` then `T` |
| `(A, B, ...)` | `T`, the element count, then each element |
| `T?` | `O` then `T` |
| `T!` | `E` then `T` |
| `fn(A, B): C` | `F`, the parameter count, each parameter, then `C` |
| `Name<A, ...>` | `N`, the name's byte length, the name, the argument count, then each argument |

Runtime entry points keep plain unmangled C names (`pump_alloc`, and so on). On
64-bit COFF there is no leading underscore.

---

## 7. Program startup

The **compiler** emits `main`; the runtime does not. That is deliberate: it
lets `pumpc` link `pump-runtime` as a Rust dependency for the JIT without the
runtime's `main` colliding with the compiler's own.

Compiler-emitted symbols:

| Symbol | Signature | Meaning |
|---|---|---|
| `main` | `int32_t main(int32_t argc, char **argv)` | the C entry point |
| `pump_module_init` | `void pump_module_init(void)` | evaluates every module constant in dependency order into its global slot |
| `pump_program_main` | `int32_t pump_program_main(void)` | calls the user's `fn main`, maps its result to an exit code, turns a pending error into a panic |
| `pump_type_table` | read-only data | the type descriptor table (section 8) |
| `pump_global_roots` | writable data | one pointer slot per module constant of reference type |

The body of `main` is fixed:

```c
int32_t main(int32_t argc, char **argv) {
    void *stack_bottom = &argc;                 /* address of a local */
    pump_rt_init(stack_bottom,
                 pump_type_table, type_count,
                 pump_global_roots, root_count,
                 argc, argv);
    pump_module_init();
    int32_t code = pump_program_main();
    pump_rt_shutdown(code);
    return code;
}
```

`type_count` and `root_count` are compile-time constants the backend
materialises inline.

The JIT (`src/jit.rs`) performs the same four calls directly rather than going
through `main`.

---

## 8. The type descriptor table

`pump_type_table` is a read-only array of 48-byte entries **indexed by type
id**: entry `n` starts at byte `n * 48`. Entries 0 through 15 are reserved and
must be present, describing the builtin shapes of section 3.

```
   +0    4  kind             u32    DescriptorKind
   +4    4  flags            u32    DESC_FLAG_*
   +8    8  size             u64    fixed instance size, or 0 if per-instance
  +16    4  ref_count        u32    entries in ref_offsets
  +20    4  variant_count    u32    entries in variants; 0 unless kind is Enum
  +24    8  ref_offsets      ptr    const u32*, ascending byte offsets, or null
  +32    8  variants         ptr    const VariantDescriptor*, or null
  +40    8  name             ptr    NUL-terminated type name, for diagnostics
```

Variant descriptor, 24 bytes:

```
   +0    4  ref_count        u32
   +4    4  reserved         u32    must be 0
   +8    8  ref_offsets      ptr    const u32*, or null
  +16    8  name             ptr    NUL-terminated variant name
```

Descriptor kinds and what each tells the collector to trace:

| Value | Kind | Trace rule |
|---|---|---|
| 0 | `Struct` | the fixed offsets in `ref_offsets` |
| 1 | `Enum` | read `tag` at `+16`, then that variant's `ref_offsets` |
| 2 | `Tuple` | as `Struct` |
| 3 | `Array` | mark `data` at `+32`; if `ELEM_IS_REF`, trace `length` slots at `data + 16 + i*8` |
| 4 | `Map` | mark `entries` and `index`; walk `entry_used` entries, skip `hash == 0`, trace the key slot if `KEY_IS_REF` and the value slot if `VALUE_IS_REF` |
| 5 | `Set` | as `Map`, ignoring the value column |
| 6 | `Closure` | trace `capture_count` slots at `+32 + i*8`; **never** trace `code` |
| 7 | `String` | nothing |
| 8 | `Box` | the slot at `+16`, if `ELEM_IS_REF` |
| 9 | `Interface` | the slot at `+24`; **never** trace `itable` at `+16` |
| 10 | `Buffer` | nothing; the owner traces the contents |

Descriptor flags:

| Bit | Name | Meaning |
|---|---|---|
| 0 | `DESC_FLAG_ELEM_IS_REF` | an array's or box's slot holds a pointer |
| 1 | `DESC_FLAG_KEY_IS_REF` | a map's or set's key slot holds a pointer |
| 2 | `DESC_FLAG_VALUE_IS_REF` | a map's value slot holds a pointer |

A set stores its elements in the **key** column of a map-shaped object, so a
set descriptor's canonical element flag is `DESC_FLAG_KEY_IS_REF`. An emitter
may set `DESC_FLAG_ELEM_IS_REF` as well - the compiler does, since a set reads
as an element container in source - and the collector traces a set's key column
when **either** bit is set. No other flag has two spellings.

`size` is 0 for `String`, `Closure` and `Buffer`, whose instances vary in size;
for those the collector reads `size` from the object header instead.

---

## 9. Garbage collection

Conservative mark-sweep, stop-the-world, single-threaded. Conservative on the
**stack and registers** only; the **heap is traced precisely** using the type
descriptors of section 8, which keeps false retention bounded to what a stack
word happens to look like.

### 9.1 Roots

1. **The stack.** From the current stack pointer up to the `stack_bottom`
   recorded by `pump_rt_init`, every 8-byte aligned word is examined.
2. **The registers.** Before scanning, `pump_gc_collect` spills the callee-saved
   register file into a stack-local buffer that lies within the scanned range.
   Combined with the platform ABI, this is sufficient: at any call site, a live
   pointer is either in a callee-saved register (hence spilled into some frame
   within the scanned range, or into the buffer) or already spilled to the
   caller's frame.
3. **The global roots table**, `pump_global_roots`: one pointer slot per module
   constant of reference type.
4. **`pump_error_slot`**, the pending-error global.
5. Any slot registered by `pump_gc_add_root`.

A stack word is treated as a pointer when it points into a live heap page and
lands exactly on an object start. Interior pointers are **not** honoured:
generated code must always keep a pointer to an object's header, never to its
payload or to an element inside it. This is a hard requirement on the backend.

### 9.2 Marking

Mark bit is `FLAG_MARK` in the object header. Marking sets the bit and pushes
the object; an already-marked object is not pushed again. Tracing then follows
the object's descriptor exactly as section 8 specifies.

Because a buffer's descriptor traces nothing, a buffer marked conservatively
and later reached through its owner is still traced correctly: the *owner*
pushes the key, value and element pointers directly, so nothing depends on the
buffer being visited a second time.

### 9.3 Sweeping

An unmarked, non-`IMMORTAL` object is freed. Its `type_id` is set to
`TYPE_ID_INVALID` and its storage returns to the allocator. A marked object has
its mark bit cleared. `IMMORTAL` objects are never freed, and their mark bit is
cleared like any other.

### 9.4 Safe points

A collection can happen only inside `pump_alloc`, `pump_alloc_buffer` or an
explicit `pump_gc_collect`. Generated code needs no safe-point polling and no
write barriers.

---

## 10. Runtime entry points

Every one uses the platform C calling convention and an unmangled name. The
authoritative machine signature of each is `RuntimeFn::signature` in
`src/abi.rs`; the C prototypes below are its prose form.

Parameters named `*out_...` are `uint64_t` out-parameters written by the callee.
`PumpString`, `PumpArray`, `PumpMap`, `PumpSet`, `PumpBox`, `PumpClosure` and
`PumpInterface` all mean "pointer to an object laid out as section 4 says".

### Lifecycle

```c
void pump_rt_init(void *stack_bottom,
                  const void *type_table, uint64_t type_count,
                  void **global_roots, uint64_t root_count,
                  int32_t argc, const char **argv);
void pump_rt_shutdown(int32_t exit_code);
```

### Allocation and collection

```c
void *pump_alloc(uint32_t type_id, uint64_t size);   /* zeroed, header written */
void *pump_alloc_buffer(uint64_t payload_size);      /* TYPE_ID_BUFFER, zeroed */
void  pump_gc_collect(void);
void  pump_gc_disable(void);
void  pump_gc_enable(void);
void  pump_gc_add_root(void **slot);
void  pump_gc_remove_root(void **slot);
```

`pump_alloc` takes the **total** size including the header, already rounded to
16, and writes `type_id`, zero flags and that size into the header. The payload
is zeroed.

### Panics - all of these never return

```c
void pump_panic(const PumpString *message);
void pump_panic_cstr(const uint8_t *bytes, uint64_t len);
void pump_panic_index(int64_t index, int64_t length);
void pump_panic_divide_by_zero(void);
void pump_panic_null(void);
void pump_panic_negative_shift(int64_t count);
void pump_panic_missing_key(void);
void pump_panic_concurrent_modification(void);
void pump_exit(int32_t code);
```

The backend must place an unreachable terminator after any call to these.

### The pending-error slot

```c
void  pump_error_set(void *error);   /* an interface value */
void *pump_error_take(void);         /* returns and clears */
int8_t pump_error_pending(void);     /* 1 when the slot is non-null */
```

### Strings

```c
PumpString *pump_string_new(const uint8_t *bytes, uint64_t len);  /* copies */
PumpString *pump_string_concat(const PumpString *a, const PumpString *b);
int8_t      pump_string_eq(const PumpString *a, const PumpString *b);
int64_t     pump_string_cmp(const PumpString *a, const PumpString *b);
uint64_t    pump_string_hash(PumpString *s);
uint64_t    pump_string_len(const PumpString *s);
uint64_t    pump_string_char_count(const PumpString *s);
int64_t     pump_string_byte_at(const PumpString *s, int64_t index);
PumpString *pump_string_slice(const PumpString *s, int64_t start, int64_t end);
PumpArray  *pump_string_chars(const PumpString *s);
PumpString *pump_string_from_char(uint32_t c);
PumpString *pump_string_from_bool(int8_t b);
PumpString *pump_string_from_int(int64_t v);
PumpString *pump_string_from_uint(uint64_t v);
PumpString *pump_string_from_float(double v);
uint32_t    pump_char_from_uint(uint64_t v);
```

`pump_string_new` copies its input and the caller guarantees valid UTF-8.
`pump_string_slice` takes byte offsets and panics unless both land on a
character boundary. `pump_char_from_uint` panics unless `v` is a Unicode scalar
value (`<= 0x10FFFF`, outside `0xD800..0xDFFF`).

### Output

```c
void pump_print(const PumpString *s);        /* stdout, no newline */
void pump_println(const PumpString *s);      /* stdout, with newline */
void pump_print_error(const PumpString *s);  /* stderr, with newline */
```

### Arrays

```c
PumpArray *pump_array_new(uint32_t type_id, uint64_t capacity);
PumpArray *pump_array_with_length(uint32_t type_id, uint64_t length); /* zeroed */
uint64_t   pump_array_len(const PumpArray *a);
uint64_t   pump_array_get(const PumpArray *a, int64_t index);  /* bounds-checked */
void       pump_array_set(PumpArray *a, int64_t index, uint64_t value);
void       pump_array_push(PumpArray *a, uint64_t value);
uint64_t   pump_array_pop(PumpArray *a);
void       pump_array_reserve(PumpArray *a, uint64_t capacity);
PumpArray *pump_array_concat(const PumpArray *a, const PumpArray *b);
PumpArray *pump_array_slice(const PumpArray *a, int64_t start, int64_t end);
```

The backend may inline `get` and `set` as a bounds check plus a load or store at
`data + 16 + index*8`, and must use `pump_panic_index` for the failure path so
the message is identical either way.

### Maps

```c
PumpMap  *pump_map_new(uint32_t type_id);
uint64_t  pump_map_len(const PumpMap *m);
int8_t    pump_map_lookup(const PumpMap *m, uint64_t key, uint64_t *out_value);
uint64_t  pump_map_get(const PumpMap *m, uint64_t key);   /* panics if absent */
void      pump_map_set(PumpMap *m, uint64_t key, uint64_t value);
int8_t    pump_map_remove(PumpMap *m, uint64_t key);
int8_t    pump_map_has(const PumpMap *m, uint64_t key);
PumpArray *pump_map_keys(const PumpMap *m);     /* insertion order */
PumpArray *pump_map_values(const PumpMap *m);   /* insertion order */
int8_t    pump_map_iter_next(const PumpMap *m, uint64_t *cursor,
                             uint64_t *out_key, uint64_t *out_value);
```

`pump_map_new` reads the slot flags from the descriptor named by `type_id` and
writes them into the new object. It does **not** read a key kind: the
descriptor has none, and the runtime derives one per operation by the rule in
section 4.3.

`pump_map_iter_next` starts with `*cursor == 0`, advances it past the entry it
returned, and returns 0 when iteration is done.

### Sets

```c
PumpSet *pump_set_new(uint32_t type_id);
uint64_t pump_set_len(const PumpSet *s);
int8_t   pump_set_add(PumpSet *s, uint64_t element);     /* 1 if newly inserted */
int8_t   pump_set_has(const PumpSet *s, uint64_t element);
int8_t   pump_set_remove(PumpSet *s, uint64_t element);
int8_t   pump_set_iter_next(const PumpSet *s, uint64_t *cursor,
                            uint64_t *out_element);
```

### Iteration guard

```c
uint64_t pump_collection_modcount(const void *collection);
```

Reads the modification counter of an array, a map or a set, dispatching on the
descriptor kind. A `for` loop snapshots it before the loop and compares each
iteration.

### Composite constructors

```c
PumpClosure  *pump_closure_new(uint32_t type_id, void *code, uint64_t capture_count);
PumpBox      *pump_box_new(uint32_t type_id, uint64_t value);
PumpInterface *pump_iface_new(const void *itable, void *data);
```

### Files, the command line, and processes

```c
PumpString *pump_read_file_text(const PumpString *path);   /* NULL on failure */
PumpArray  *pump_read_file_bytes(const PumpString *path);  /* NULL on failure */
int8_t      pump_write_file_text(const PumpString *path, const PumpString *data);
int8_t      pump_write_file_bytes(const PumpString *path, const PumpArray *data);
PumpArray  *pump_os_args(void);                            /* [string], argv[0] first */
PumpBox    *pump_os_run(const PumpString *program, const PumpArray *arguments);
PumpString *pump_os_error(void);
```

**These seven do not use `pump_error_slot`, and that is deliberate.** A slot
value has to be an `Error` interface value, whose itable the compiler emits;
the runtime cannot build one. So they report failure the way C does: a null
pointer or a `0`, with the reason left in a slot of their own that
`pump_os_error` reads back as a `string`. Every one of them CLEARS that slot
on entry, so `pump_os_error` returns `""` exactly when the most recent of
these calls succeeded. The `T!` that Pump code actually sees is built on top,
in `std/io.pump` and `std/os.pump`, which read the slot and `fail`.

None of them panic. A missing file must be catchable.

`pump_read_file_bytes` gives one array element per byte, each 0..255, and
`pump_write_file_bytes` refuses - 0 with a message, no partial write - an
element outside that range rather than truncating it. Pump has no `u8`, so a
byte occupies a whole 64-bit slot; this is eight times the memory the data
needs and is the price of correctness until sized integers arrive.

`pump_os_run` returns an `int?` per section 2.1: a `TYPE_ID_BOX_SCALAR` box
holding the exit code, or null when the program never started. A program that
ran and exited non-zero is a success here, and its code comes back in the box.
It flushes stdout before spawning, since the child inherits it.

---

## 11. Rules the backend must not break

These are the invariants the collector's soundness rests on. Each of them is
invisible in the generated code that violates it and fatal at a random later
allocation.

1. **A live pointer always points at an object header.** Never keep only an
   interior pointer - not to a payload, not to an element, not one-past-the-end.
   Compute an element address, use it, and discard it without an allocation in
   between; if an allocation can intervene, keep the owning object pointer live
   and recompute.
2. **Never write `FLAG_MARK`.** Only the collector touches it.
3. **`size` in the header is exact and 16-aligned**, for variable-length
   objects too.
4. **Pass `pump_alloc` the total size including the header**, already rounded.
5. **Interface `itable` and closure `code` are never GC pointers.** Do not put
   a GC-allocated object in either slot.
6. **The descriptor's `ref_offsets` must list every pointer field and nothing
   else**, ascending. A missing offset frees a live object; a spurious one
   dereferences an integer.
7. **After a call to a diverging runtime function, emit an unreachable
   terminator.** Falling through returns to nowhere.
8. **After a failable call, test `pump_error_slot` before using the result.**
   The returned value is meaningless when an error is in flight.
