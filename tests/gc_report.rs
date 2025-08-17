// GC ton bao nhieu. Do that chu khong doan.
//
// Ba con so quyet dinh xem viet mot bo mark-sweep bao thu dung ca the gioi co
// phai la mot y hop ly khong, va ca ba deu re de do bay gio va dat de do sau:
//
//  1. thoi gian dung, va no ti le voi cai gi. Mark ti le voi phan con song,
//     sweep ti le voi ca heap.
//  2. heap co phinh ra sau mot lan chay dai khong. Bo GC nao thu lai gan het
//     ma phan can lai cu tang deu thi van la ro ri.
//  3. giu nham - rac bi giu lai vi mot tu tren stack tinh co giong dia chi
//     cua no. Day la cai gia cua viec quet stack kieu bao thu thay vi tin vao
//     stack map cua Cranelift.
//
// Moi test o duoi tu in bang cua no ra. Chay bang:
//
//   cargo test --release --test gc_report -- --nocapture
//
// May cai assert co y noi long: chung o day de bat mot bo GC da hong, chu
// khong phai de ghim mot con so thoi gian vao mot cai may cu the. Duoi day la
// nhung gi may t do duoc - x86_64, Windows 11, --release, mot luong - de sau
// nay co cai ma so.
//
// -- thoi gian dung --
//
//   object song | song MiB | heap MiB | dung trung vi | moi object
//   ------------|----------|----------|---------------|-----------
//   1 000       | 0.05     | 1.00     | 0.01 ms       | 11 ns
//   10 000      | 0.46     | 1.00     | 0.11 ms       | 11 ns
//   100 000      | 2.29     | 3.00     | 0.68 ms       | 14 ns
//   200 000     | 9.16     | 10.00    | 3.49 ms       | 17 ns
//
// Tuyen tinh theo phan con song, khoang 11 ns mot object khi heap con vua
// cache va 17 ns khi khong vua nua. Cai can nho la HINH DANG chu khong phai
// con so: mot lan dung bi chan boi bao nhieu thu con song, chu khong phai boi
// bao nhieu rac da chat dong. Cung bang do ma build khong toi uu thi doc ra
// 0.11, 1.75, 5.57 va 31.11 ms, cham gap nam den muoi lan.
//
// -- heap phinh --
//
// Hai tram vong, moi vong cap 20 nghin object roi bo het, tren mot phan song
// 10 nghin: bon trieu lan cap phat, 333 lan gom, 380 ms. Bo nho dat truoc la
// 1.00 MiB sau vong dau, o giua, o cuoi, va o dinh. No khong nhuc nhich. Gap
// ba muoi hai lan kich thuoc heap da chay qua no ma khong de lai gi.
//
// -- giu nham --
//
//   hinh rac                     | dung   | giu lai | ty le
//   -----------------------------|--------|---------|-------
//   20 000 object roi            | 20 000 | 0       | 0.00%
//   200 day dai 100              | 20 000 | 2       | 0.01%
//   20 day dai 1 000             | 20 000 | 2       | 0.01%
//   100 vong dai 200             | 20 000 | 200     | 1.00%
//   mot lan de quy 1 000 frame   | 2 002  | 4       | 0.20%
//
// Cai hinh quan trong hon con so, va no dung nhu thiet ke doan truoc. So goc
// gia thi be ti, mot hai tu tren stack, nhung moi cai giu lai TAT CA nhung gi
// tu no voi toi duoc. Rac roi thi thu lai sach; mot tu cu tro vao mot cai
// vong 200 node thi giu ca 200. Nen cai gia cua quet bao thu khong phai la
// "may phan tram cua heap", ma la "mot vai nhanh", va nhanh do to bao nhieu
// thi tuy chuong trinh.
//
// No khong don lai. false_retention_does_not_accumulate dung dung 20 nghin
// node vong do muoi hai lan va lan nao cung thay dung 200 cai song sot, khong
// bao gio 400.

mod gcsupport;

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use pump_runtime::alloc::{heap_bytes_in_use, heap_bytes_reserved, heap_object_count};
use pump_runtime::gc::{collection_count, pump_gc_collect};

use gcsupport::{clobber_stack, leaf, node, set_slot, slot, with_runtime, Roots, CHILD, NEXT};

// ===== may hinh dung chung =====

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

fn mib(bytes: usize) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn spread(samples: &mut [Duration]) -> (Duration, Duration, Duration) {
    samples.sort_unstable();
    (
        samples[0],
        samples[samples.len() / 2],
        samples[samples.len() - 1],
    )
}

// ===== thoi gian dung =====
