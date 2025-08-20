// GC, soi tu ben trong runtime.
//
// tests/gc_cases.rs chay ca chuong trinh Pump va cham diem GC qua nhung gi
// chung in ra. Cach do bat duoc bo GC nao gom nham do con song, nhung no
// khong nhin thay heap: mot cai vong co thu lai that khong, heap co ngung
// phinh khong, byte cua thu song sot co dung la byte cu khong. May test o day
// link thang pump-runtime va kiem may cai do.
//
// Cai to nhat la a_mutating_graph_is_traced_exactly: no giu mot ban mo phong
// do thi object o phia Rust, sua do thi that va ban mo phong cung mot luc, va
// sau moi lan gom thi di het ban mo phong de khang dinh moi object no bao la
// voi toi duoc thi van con nguyen va van tro dung cho.

mod gcsupport;

use pump_runtime::alloc::{
    heap_bytes_in_use, heap_bytes_reserved, heap_object_count, MIN_HEAP_BYTES,
};
use pump_runtime::gc::{collection_count, pump_gc_collect, pump_gc_disable, pump_gc_enable};

use gcsupport::{
    assert_intact, clobber_stack, leaf, node, set_slot, slot, with_runtime, word, Rng, Roots,
    CHILD, NEXT, VALUE,
};

#[inline(never)]
fn churn(count: usize) {
    for index in 0..count {
        std::hint::black_box(leaf(index as u64));
    }
}

fn chain(length: u64) -> *mut u8 {
    let head = node(0);
    let mut previous = head;
    for index in 1..length {
        let next = node(index);
        set_slot(previous, NEXT, next);
        previous = next;
    }
    head
}

fn verify_chain(head: *mut u8, expected: u64, context: &str) {
    let mut cursor = head;
    let mut index = 0u64;
    while !cursor.is_null() {
        assert_intact(cursor, index, context);
        cursor = slot(cursor, NEXT);
        index += 1;
    }
    assert_eq!(index, expected, "{context}: the chain lost links");
}

#[inline(never)]
fn ring(length: u64) -> *mut u8 {
    let head = chain(length);
    let mut tail = head;
    while !slot(tail, NEXT).is_null() {
        tail = slot(tail, NEXT);
    }
    set_slot(tail, NEXT, head);
    head
}

// ===== con song hay khong =====

#[test]
fn a_rooted_chain_survives_every_collection() {
    with_runtime(|| {
        const LENGTH: u64 = 20_000;
        let mut roots = Roots::new(1);
        roots.set(0, chain(LENGTH));
        clobber_stack();

        for round in 0..8 {
            churn(20_000);
            pump_gc_collect();
            verify_chain(roots.get(0), LENGTH, &format!("after round {round}"));
        }

        assert!(
            collection_count() >= 8,
            "the loop never provoked a collection"
        );
    });
}
