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

#[test]
fn a_survivors_storage_is_never_handed_out_again() {
    with_runtime(|| {
        const COUNT: usize = 4_000;
        let mut roots = Roots::new(COUNT);
        for index in 0..COUNT {
            roots.set(index, node(index as u64));
        }
        clobber_stack();

        for round in 0..6 {
            pump_gc_collect();
            churn(50_000);
            for index in 0..COUNT {
                assert_intact(
                    roots.get(index),
                    index as u64,
                    &format!("after round {round}"),
                );
            }
        }
    });
}

// ===== vong tron =====

#[inline(never)]
fn bu_cycles(count: usize, length: u64) {
    for _ in 0..count {
        let head = ring(length);
        let self_loop = node(0);
        set_slot(self_loop, NEXT, self_loop);
        set_slot(self_loop, CHILD, head);
        std::hint::black_box(self_loop);
    }
}

#[test]
fn unreachable_cycles_are_reclaimed() {
    with_runtime(|| {
        const RINGS: usize = 30;
        const LENGTH: u64 = 200;
        const BUILT: usize = RINGS * (LENGTH as usize + 1);

        pump_gc_collect();
        let before = heap_object_count();

        bu_cycles(RINGS, LENGTH);
        assert!(
            heap_object_count() >= before + BUILT,
            "the rings were not built"
        );

        clobber_stack();
        pump_gc_collect();

        // vong thi lien thong manh, nen chi mot tu cu tren stack tinh co
        // giong dia chi mot node la giu lai ca 201 object cua no. Chan o day
        // cho phep mot ...
        // tests/gc_report.rs bao.
        let retained = heap_object_count() - before;
        assert!(
            retained <= 2 * (LENGTH as usize + 1),
            "{retained} of {BUILT} cycle nodes were still live after a \
             collection, which is more than a stale stack word or two explains"
        );
    });
}

#[test]
fn a_reachable_cycle_survives_intact() {
    with_runtime(|| {
        const LENGTH: u64 = 500;
        let mut roots = Roots::new(1);
        roots.set(0, ring(LENGTH));
        clobber_stack();

        for round in 0..6 {
            churn(30_000);
            pump_gc_collect();

            let head = roots.get(0);
            let mut cursor = head;
            for index in 0..LENGTH {
                assert_intact(cursor, index, &format!("after round {round}"));
                cursor = slot(cursor, NEXT);
            }
            assert_eq!(cursor, head, "round {round}: the ring came unclosed");
        }
    });
}

// ===== ep cap phat =====

#[test]
fn allocation_pressure_collects_rather_than_growing() {
    with_runtime(|| {
        // Thirty-two times the collection threshold, all of it garbage.
        let objects = MIN_HEAP_BYTES / 32 * 32;
        churn(objects);
        clobber_stack();

        assert!(
            collection_count() > 0,
            "allocating {objects} objects never triggered a collection"
        );
        let reserved = heap_bytes_reserved();
        assert!(
            reserved <= MIN_HEAP_BYTES * 4,
            "the heap grew to {reserved} bytes to hold nothing but garbage"
        );
    });
}

#[test]
fn a_long_run_with_a_steady_live_set_stops_growing() {
    with_runtime(|| {
        const LENGTH: u64 = 5_000;
        let mut roots = Roots::new(1);
        roots.set(0, chain(LENGTH));
        clobber_stack();

        // Two warm-up rounds, so the measurement starts from a settled heap.
        for _ in 0..2 {
            churn(40_000);
            pump_gc_collect();
        }
        let settled = heap_bytes_reserved();

        for round in 0..40 {
            churn(40_000);
            pump_gc_collect();
            assert_eq!(
                heap_bytes_reserved(),
                settled,
                "round {round} grew the heap past its settled size"
            );
        }
        verify_chain(roots.get(0), LENGTH, "after forty rounds");
    });
}
