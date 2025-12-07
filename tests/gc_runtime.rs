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
fn build_cycles(count: usize, length: u64) {
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

        build_cycles(RINGS, LENGTH);
        assert!(
            heap_object_count() >= before + BUILT,
            "the rings were not built"
        );

        clobber_stack();
        pump_gc_collect();

        // vong thi lien thong manh, nen chi mot tu cu tren stack tinh co
        // giong dia chi mot node la giu lai ca 201 object cua no. Chan o day
        // cho phep mot hai cai the, khong hon; con so that thi
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

#[test]
fn large_objects_are_reclaimed_and_their_space_reused() {
    with_runtime(|| {
        use pump_runtime::alloc::pump_alloc_buffer;

        pump_gc_collect();
        let baseline = heap_bytes_reserved();

        for round in 0..40 {
            for _ in 0..16 {
                std::hint::black_box(pump_alloc_buffer(64 * 1024));
            }
            clobber_stack();
            pump_gc_collect();
            assert!(
                heap_bytes_reserved() <= baseline.max(MIN_HEAP_BYTES) * 4,
                "round {round}: forty megabytes of dead buffers grew the heap to {} bytes",
                heap_bytes_reserved()
            );
        }
    });
}

// ===== quet stack kieu bao thu =====

fn descend(depth: u64, remaining: u64) -> u64 {
    let mine = node(depth);
    // A second object, linked from the first, so the scan has to find a root
    // and the trace has to follow it.
    set_slot(mine, CHILD, node(depth + 1_000_000));

    if remaining == 0 {
        churn(60_000);
        pump_gc_collect();
        assert_intact(mine, depth, "at the bottom of the descent");
        return depth;
    }

    let below = descend(depth + 1, remaining - 1);

    assert_intact(mine, depth, "on the way out of the descent");
    let child = slot(mine, CHILD);
    assert_intact(child, depth + 1_000_000, "a child on the way out");
    below + depth
}

#[test]
fn deep_recursion_keeps_every_frames_roots_alive() {
    with_runtime(|| {
        const DEPTH: u64 = 1_500;
        let total = descend(0, DEPTH);
        assert_eq!(total, DEPTH * (DEPTH + 1) / 2);
    });
}

#[test]
fn a_reference_held_only_in_a_register_survives() {
    with_runtime(|| {
        let kept = std::hint::black_box(node(7));
        for _ in 0..4 {
            churn(40_000);
            pump_gc_collect();
            assert_intact(std::hint::black_box(kept), 7, "a register-held node");
        }
    });
}

#[test]
fn every_live_object_is_found_by_the_stack_scan() {
    use gcsupport::node_of_size;

    with_runtime(|| {
        const COUNT: usize = 1_500;
        const SIZES: [u64; 8] = [48, 64, 128, 512, 1_024, 2_048, 8_192, 65_536];

        // Fragment the heap first, so what follows comes from a mixture of
        // split free blocks and fresh bump space rather than one clean run.
        churn(60_000);
        clobber_stack();
        pump_gc_collect();

        // A stack array is the point: the only reference to any of these
        // objects is a word in this frame, so each one survives only if the
        // conservative scan recognises its address.
        let mut held = [std::ptr::null_mut::<u8>(); COUNT];
        for (index, entry) in held.iter_mut().enumerate() {
            *entry = node_of_size(index as u64, SIZES[index % SIZES.len()]);
        }

        pump_gc_collect();
        for (index, &object) in held.iter().enumerate() {
            assert_intact(object, index as u64, "held only on the stack");
        }

        // Drop every other one, collect, and refill the holes. The survivors
        // must come through a sweep that freed and coalesced around them.
        for entry in held.iter_mut().step_by(2) {
            *entry = std::ptr::null_mut();
        }
        pump_gc_collect();
        churn(40_000);
        for (index, entry) in held.iter_mut().enumerate() {
            if entry.is_null() {
                *entry = node_of_size(index as u64, SIZES[index % SIZES.len()]);
            }
        }

        pump_gc_collect();
        for (index, &object) in held.iter().enumerate() {
            assert_intact(object, index as u64, "after a sweep around it");
        }
        std::hint::black_box(&held);
    });
}

// ===== tam dung gom =====

#[test]
fn a_disabled_collector_defers_and_a_re_enabled_one_catches_up() {
    with_runtime(|| {
        pump_gc_collect();
        let collections = collection_count();

        pump_gc_disable();
        churn(MIN_HEAP_BYTES / 32 * 4);
        assert_eq!(
            collection_count(),
            collections,
            "a disabled collector ran anyway"
        );
        let while_disabled = heap_bytes_in_use();

        pump_gc_enable();
        clobber_stack();
        pump_gc_collect();

        assert_eq!(collection_count(), collections + 1);
        assert!(
            heap_bytes_in_use() * 8 < while_disabled,
            "re-enabling the collector reclaimed almost nothing: {} bytes of {}",
            heap_bytes_in_use(),
            while_disabled
        );
    });
}

#[test]
fn suspension_nests() {
    with_runtime(|| {
        pump_gc_collect();
        let collections = collection_count();

        pump_gc_disable();
        pump_gc_disable();
        pump_gc_enable();
        churn(MIN_HEAP_BYTES / 32 * 4);
        assert_eq!(collection_count(), collections, "one enable was enough");

        pump_gc_enable();
        churn(MIN_HEAP_BYTES / 32 * 4);
        assert!(
            collection_count() > collections,
            "the second enable did not"
        );
    });
}

// ===== do thi bi sua lien tuc, so voi mot ban mo phong =====

struct Model {
    addresses: Vec<*mut u8>,
    edges: Vec<[Option<usize>; 2]>,
    roots: Roots,
    root_of: Vec<Option<usize>>,
}

impl Model {
    fn new(root_count: usize) -> Model {
        Model {
            addresses: Vec::new(),
            edges: Vec::new(),
            roots: Roots::new(root_count),
            root_of: vec![None; root_count],
        }
    }

    fn add(&mut self) -> usize {
        let index = self.addresses.len();
        self.addresses.push(node(index as u64));
        self.edges.push([None, None]);
        index
    }

    fn grow(&mut self, count: usize) {
        pump_gc_disable();
        for _ in 0..count {
            self.add();
        }
        pump_gc_enable();
    }

    fn link(&mut self, from: usize, field: usize, to: Option<usize>) {
        let offset = if field == 0 { NEXT } else { CHILD };
        let target = to.map_or(std::ptr::null_mut(), |index| self.addresses[index]);
        set_slot(self.addresses[from], offset, target);
        self.edges[from][field] = to;
    }

    fn root(&mut self, position: usize, index: Option<usize>) {
        let target = index.map_or(std::ptr::null_mut(), |index| self.addresses[index]);
        self.roots.set(position, target);
        self.root_of[position] = index;
    }

    fn reachable(&self) -> Vec<usize> {
        let mut seen = vec![false; self.addresses.len()];
        let mut pending: Vec<usize> = self.root_of.iter().flatten().copied().collect();
        let mut found = Vec::new();
        while let Some(index) = pending.pop() {
            if std::mem::replace(&mut seen[index], true) {
                continue;
            }
            found.push(index);
            for target in self.edges[index].iter().flatten() {
                pending.push(*target);
            }
        }
        found
    }

    fn verify(&self, reachable: &[usize], context: &str) {
        for &index in reachable {
            let object = self.addresses[index];
            assert_intact(object, index as u64, context);
            for (field, offset) in [(0usize, NEXT), (1usize, CHILD)] {
                let expected = self.edges[index][field]
                    .map_or(std::ptr::null_mut(), |target| self.addresses[target]);
                let found = slot(object, offset);
                assert_eq!(
                    found, expected,
                    "{context}: node {index} field {field} points at {found:p}, \
                     and the model says {expected:p}"
                );
            }
        }
    }
}

#[test]
fn a_mutating_graph_is_traced_exactly() {
    with_runtime(|| {
        const ROOTS: usize = 24;
        const INITIAL: usize = 4_000;
        const ROUNDS: usize = 30;

        let mut rng = Rng::new(0x5eed_1234);
        let mut model = Model::new(ROOTS);

        model.grow(INITIAL);
        for index in 0..INITIAL {
            let first = rng.below(INITIAL);
            let second = rng.below(INITIAL);
            model.link(index, 0, Some(first));
            model.link(index, 1, Some(second));
        }
        for position in 0..ROOTS {
            let index = rng.below(INITIAL);
            model.root(position, Some(index));
        }

        let mut worst_false_retention = 0usize;
        for round in 0..ROUNDS {
            let live = model.reachable();
            assert!(!live.is_empty(), "round {round}: the graph emptied");

            pump_gc_disable();
            for _ in 0..live.len() / 4 {
                let from = live[rng.below(live.len())];
                let field = rng.below(2);
                match rng.below(8) {
                    0 => model.link(from, field, None),
                    1 | 2 => {
                        let fresh = model.add();
                        model.link(from, field, Some(fresh));
                    }
                    _ => {
                        let to = live[rng.below(live.len())];
                        model.link(from, field, Some(to));
                    }
                }
            }
            // Move a root, which is how a whole subgraph becomes garbage.
            let position = rng.below(ROOTS);
            let target = live[rng.below(live.len())];
            model.root(position, Some(target));
            pump_gc_enable();

            let live = model.reachable();
            churn(20_000);
            clobber_stack();
            pump_gc_collect();
            model.verify(&live, &format!("round {round}"));

            let retained = heap_object_count().saturating_sub(live.len());
            worst_false_retention = worst_false_retention.max(retained);
        }

        // The number is reported properly by `tests/gc_report.rs`; the bound
        // here is only wide enough to catch a collector that has stopped
        // reclaiming rather than one having an unlucky stack.
        assert!(
            worst_false_retention < 4_000,
            "{worst_false_retention} objects survived that the model says are \
             unreachable, which is more than a conservative scan explains"
        );
    });
}

#[test]
fn dropping_every_root_reclaims_the_whole_graph() {
    with_runtime(|| {
        pump_gc_collect();
        let baseline = heap_object_count();

        let mut model = Model::new(4);
        model.grow(3_000);
        let mut rng = Rng::new(0xfeed_9876);
        for index in 0..3_000 {
            model.link(index, 0, Some(rng.below(3_000)));
            model.link(index, 1, Some(rng.below(3_000)));
        }
        for position in 0..4 {
            model.root(position, Some(rng.below(3_000)));
        }

        clobber_stack();
        pump_gc_collect();
        assert!(
            heap_object_count() >= baseline + 2_000,
            "the graph did not survive while it was rooted"
        );

        for position in 0..4 {
            model.root(position, None);
        }
        // The model's own `addresses` vector lives on the malloc heap, which
        // the collector never scans, so nothing but a stale stack word can
        // keep the graph now.
        clobber_stack();
        pump_gc_collect();

        let after = heap_object_count().saturating_sub(baseline);
        assert!(
            after <= 128,
            "{after} of 3000 nodes survived after every root was dropped"
        );
        drop(model);
    });
}

// ===== header co sach khong =====

#[test]
fn no_mark_bit_survives_the_collection_that_set_it() {
    with_runtime(|| {
        const LENGTH: u64 = 2_000;
        let mut roots = Roots::new(1);
        roots.set(0, chain(LENGTH));

        for round in 0..4 {
            pump_gc_collect();
            let mut cursor = roots.get(0);
            while !cursor.is_null() {
                assert_eq!(
                    gcsupport::flags(cursor) & pump_runtime::FLAG_MARK,
                    0,
                    "round {round}: a survivor kept its mark bit"
                );
                cursor = slot(cursor, NEXT);
            }
            // If a mark bit had survived, the next cycle would free the rest
            // of the chain behind it.
            churn(20_000);
            pump_gc_collect();
            verify_chain(roots.get(0), LENGTH, &format!("round {round}"));
        }
    });
}

#[test]
fn a_survivors_payload_is_bit_for_bit_unchanged() {
    with_runtime(|| {
        const COUNT: usize = 2_000;
        let mut roots = Roots::new(COUNT);
        for index in 0..COUNT {
            let object = node(index as u64);
            gcsupport::set_word(object, VALUE, 0xdead_0000_0000_0000 | index as u64);
            roots.set(index, object);
        }

        for round in 0..5 {
            churn(30_000);
            pump_gc_collect();
            for index in 0..COUNT {
                assert_eq!(
                    word(roots.get(index), VALUE),
                    0xdead_0000_0000_0000 | index as u64,
                    "round {round}: node {index} came back with a different payload"
                );
            }
        }
    });
}
