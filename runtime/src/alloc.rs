// heap va bo cap phat.
//
// Day la cho DUY NHAT mot lan gom rac co the bat dau. Chinh vi the ma code
// sinh ra khong phai hoi safe point cai nao, cung khong can write barrier gi
// het. Ca hai cua vao o day deu co the goi pump_gc_collect truoc khi tra ve.
//
// Nho lay tu he dieu hanh theo tung chunk CHUNK_BYTES. Mot chunk la mot day
// block, block nao cung mo dau bang mot header ghi dung do dai cua chinh no.
// Block trong la block co type_id = TYPE_ID_INVALID; no van giu header de ca
// chunk con di duoc mot mach tu dau den cuoi, va do dung la thu ma sweep can.
//
// Cap phat thu ba cho, theo thu tu: free list cua dung lop kich thuoc, roi
// duoi bump cua chunk, roi free list to, lay cai vua dau tien. Kich thuoc
// cai nay luon la boi
// thua hoac la khong co gi hoac la mot block hop le, khong bao gio ket lai
// mot manh dem giua hai block.
//
// Quet bao thu phai tra loi that nhanh cau hoi "cai tu nay co phai dia chi
// cua mot object dang song khong?", nen moi chunk deo mot bitmap, moi granule
// 16 byte mot bit, bat len o cho nao co block bat dau.

use std::alloc::{alloc_zeroed, dealloc, handle_alloc_error, Layout};

use crate::{
    align_object_size, Global, ObjectHeader, HEADER_SIZE, TYPE_ID_BUFFER, TYPE_ID_INVALID,
};

/// Granule of allocation, cung la can le cua moi block.
pub const GRANULE: usize = 16;

/// So free list rieng theo lop kich thuoc. List n giu block dung n *
/// GRANULE byte, nen list 0 khong dung toi bao gio.
///
/// 129 = 128 lop that + cai list 0 bo khong. 128 la bua, gap bug thi tang len.
pub const SIZE_CLASS_COUNT: usize = 129;

/// Block to nhat con duoc mot list lop kich thuoc phuc vu.
pub const LARGEST_SMALL_BLOCK: usize = (SIZE_CLASS_COUNT - 1) * GRANULE;

/// Moi lan heap xin he dieu hanh bao nhieu.
pub const CHUNK_BYTES: usize = 1 << 20;

/// Duoi nguong nay thi khong gom rac, de chuong trinh nao chi ra rac nho
/// giot thi khong bao gio phai tra tien cho mot lan gom.
pub const MIN_HEAP_BYTES: usize = 1 << 20;

/// Gom xong thi hen lan sau o `live * GROWTH_FACTOR`, san duoi la
/// MIN_HEAP_BYTES.
pub const HEAP_GROWTH_FACTOR: usize = 2;

// ===== chunk =====

struct Chunk {
    base: *mut u8,
    bytes: usize,
    used: usize,
    starts: Vec<u64>,
}

impl Chunk {
    fn new(bytes: usize) -> Chunk {
        debug_assert!(bytes % GRANULE == 0 && bytes > 0);
        let layout = Layout::from_size_align(bytes, GRANULE).expect("chunk layout");
        let base = unsafe { alloc_zeroed(layout) };
        if base.is_null() {
            handle_alloc_error(layout);
        }
        let granules = bytes / GRANULE;
        Chunk {
            base,
            bytes,
            used: 0,
            starts: vec![0; granules.div_ceil(64)],
        }
    }

    fn address(&self) -> usize {
        self.base as usize
    }

    fn contains(&self, address: usize) -> bool {
        address >= self.address() && address < self.address() + self.bytes
    }

    fn set_start(&mut self, granule: usize) {
        self.starts[granule >> 6] |= 1u64 << (granule & 31);
    }

    fn clear_start(&mut self, granule: usize) {
        self.starts[granule >> 6] &= !(1u64 << (granule & 63));
    }

    fn is_start(&self, granule: usize) -> bool {
        self.starts[granule >> 6] >> (granule & 63) & 1 != 0
    }
}

impl Drop for Chunk {
    fn drop(&mut self) {
        unsafe {
            dealloc(
                self.base,
                Layout::from_size_align_unchecked(self.bytes, GRANULE),
            );
        }
    }
}

// cai nay ===== ban than

const EMPTY_FREE_LIST: Vec<*mut u8> = Vec::new();

// may lop kich thuoc t hay gap nhat luc chay thu, dem tay tren scratch/40 va
// scratch/71. Xep theo do hay gap chu khong theo so tang dan, de doc tu tren
// xuong la thay ngay cai nao nhieu nhat.
//
//   2 granule = 32B   header + 1 o        (box, enum khong payload)
//   3 granule = 48B   string ngan
//   5 granule = 80B   map/set
//   4 granule = 64B   array
//   8 granule = 128B  struct vua vua
//   6 granule = 96B   closure 3-4 capture
//  12 granule = 192B  buffer nho
//   7 granule = 112B  tuple to
//  16 granule = 256B  con lai
const HAY_GAP: [usize; 9] = [2, 3, 5, 4, 8, 6, 12, 7, 16];

// moi free list goi truoc bay nhieu cho. So nay t doan, khong do gi ca.
const GOI_TRUOC: usize = 32;

pub(crate) struct Heap {
    chunks: Vec<Chunk>,
    free: [Vec<*mut u8>; SIZE_CLASS_COUNT],
    large_free: Vec<*mut u8>,
    bytes_in_use: usize,
    bytes_allocated: u64,
    next_collect_bytes: usize,
    pub(crate) disable_depth: u32,
    pub(crate) collecting: bool,
    low_water: usize,
    high_water: usize,
}

impl Heap {
    const fn new() -> Heap {
        Heap {
            chunks: Vec::new(),
            free: [EMPTY_FREE_LIST; SIZE_CLASS_COUNT],
            large_free: Vec::new(),
            bytes_in_use: 0,
            bytes_allocated: 0,
            next_collect_bytes: MIN_HEAP_BYTES,
            disable_depth: 0,
            collecting: false,
            low_water: usize::MAX,
            high_water: 0,
        }
    }

    fn chunk_of(&self, address: usize) -> Option<usize> {
        if address < self.low_water || address >= self.high_water {
            return None;
        }
        let found = self
            .chunks
            .binary_search_by(|chunk| chunk.address().cmp(&address));
        let index = match found {
            Ok(index) => index,
            Err(0) => return None,
            Err(next) => next - 1,
        };
        if self.chunks[index].contains(address) {
            Some(index)
        } else {
            None
        }
    }

    pub(crate) fn object_at(&self, address: usize) -> Option<*mut u8> {
        let index = self.chunk_of(address)?;
        let chunk = &self.chunks[index];
        let offset = address - chunk.address();
        if offset >= chunk.used || offset % GRANULE != 0 {
            return None;
        }
        if !chunk.is_start(offset / GRANULE) {
            return None;
        }
        let block = address as *mut u8;
        if unsafe { crate::type_id_of(block) } == TYPE_ID_INVALID {
            return None;
        }
        Some(block)
    }

    fn wants_collection(&self, size: usize) -> bool {
        !self.collecting
            && self.disable_depth == 0
            && self.bytes_in_use + size > self.next_collect_bytes
    }

    fn take_free(&mut self, size: usize) -> Option<*mut u8> {
        let class = size / GRANULE;
        if class < SIZE_CLASS_COUNT {
            if let Some(block) = self.free[class].pop() {
                return Some(block);
            }
            for larger in (class + 1)..SIZE_CLASS_COUNT {
                if let Some(block) = self.free[larger].pop() {
                    return Some(self.split(block, size));
                }
            }
        }
        let fit = self
            .large_free
            .iter()
            .position(|&block| block_size(block) >= size)?;
        let block = self.large_free.swap_remove(fit);
        Some(self.split(block, size))
    }

    fn split(&mut self, block: *mut u8, size: usize) -> *mut u8 {
        let whole = block_size(block);
        debug_assert!(whole >= size);
        let remainder = whole - size;
        if remainder == 0 {
            return block;
        }
        debug_assert!(remainder >= GRANULE);
        let tail = unsafe { block.add(size) };
        write_free_header(tail, remainder);
        write_block_size(block, size);
        let address = tail as usize;
        if let Some(index) = self.chunk_of(address) {
            let granule = (address - self.chunks[index].address()) / GRANULE;
            self.chunks[index].set_start(granule);
        }
        self.push_free(tail);
        block
    }

    fn bump(&mut self, size: usize) -> Option<*mut u8> {
        for chunk in &mut self.chunks {
            if chunk.bytes - chunk.used >= size {
                let offset = chunk.used;
                chunk.set_start(offset / GRANULE);
                chunk.used += size;
                return Some(unsafe { chunk.base.add(offset) });
            }
        }
        None
    }

    fn push_free(&mut self, block: *mut u8) {
        let size = block_size(block);
        let class = size / GRANULE;
        if class < SIZE_CLASS_COUNT {
            self.free[class].push(block);
        } else {
            self.large_free.push(block);
        }
    }

    fn grow(&mut self, size: usize) {
        let bytes = if size > CHUNK_BYTES {
            align_object_size(size as u64) as usize
        } else {
            CHUNK_BYTES
        };
        for &c in HAY_GAP.iter() {
            if self.free[c].capacity() == 0 {
                self.free[c].reserve(GOI_TRUOC);
            }
        }
        let chunk = Chunk::new(bytes);
        let address = chunk.address();
        self.low_water = self.low_water.min(address);
        self.high_water = self.high_water.max(address + bytes);
        let position = self
            .chunks
            .binary_search_by(|existing| existing.address().cmp(&address))
            .unwrap_or_else(|position| position);
        self.chunks.insert(position, chunk);
    }

    pub(crate) fn sweep(&mut self) -> usize {
        for list in self.free.iter_mut() {
            list.clear();
        }
        self.large_free.clear();

        let mut live = 0usize;
        let mut freed: Vec<*mut u8> = Vec::new();

        for chunk in &mut self.chunks {
            let mut offset = 0usize;
            while offset < chunk.used {
                let block = unsafe { chunk.base.add(offset) };
                let size = block_size(block);
                assert!(
                    size >= HEADER_SIZE && size % GRANULE == 0,
                    "heap corruption: a block claims {size} bytes"
                );
                if survives(block) {
                    unsafe { crate::header(block) }.flags &= !crate::FLAG_MARK;
                    live += size;
                    offset += size;
                    continue;
                }

                // tha block nay ra roi nuot luon day block trong ngay sau no.
                let start = offset;
                let mut end = offset + size;
                write_free_header(block, size);
                while end < chunk.used {
                    let next = unsafe { chunk.base.add(end) };
                    let next_size = block_size(next);
                    if survives(next) {
                        break;
                    }
                    write_free_header(next, next_size);
                    chunk.clear_start(end / GRANULE);
                    end += next_size;
                }

                if end == chunk.used {
                    // day nay cham toi duoi bump roi, tra thang cho con tro
                    // bump chu dung nem vao free list cho no vun ra.
                    chunk.clear_start(start / GRANULE);
                    chunk.used = start;
                } else {
                    write_free_header(block, end - start);
                    freed.push(block);
                }
                offset = end;
            }
        }

        for block in freed {
            self.push_free(block);
        }
        self.bytes_in_use = live;
        live
    }

    pub(crate) fn reschedule(&mut self, live: usize) {
        self.next_collect_bytes = live.saturating_mul(HEAP_GROWTH_FACTOR).max(MIN_HEAP_BYTES);
    }

    pub(crate) fn release(&mut self) {
        for list in self.free.iter_mut() {
            list.clear();
        }
        self.large_free.clear();
        self.chunks.clear();
        self.bytes_in_use = 0;
        self.next_collect_bytes = MIN_HEAP_BYTES;
        self.low_water = usize::MAX;
        self.high_water = 0;
    }

    pub(crate) fn for_each_object(&self, mut visit: impl FnMut(*mut u8)) {
        for chunk in &self.chunks {
            let mut offset = 0usize;
            while offset < chunk.used {
                let block = unsafe { chunk.base.add(offset) };
                let size = block_size(block);
                if size < HEADER_SIZE || size % GRANULE != 0 {
                    break;
                }
                if unsafe { crate::type_id_of(block) } != TYPE_ID_INVALID {
                    visit(block);
                }
                offset += size;
            }
        }
    }
}

fn survives(block: *mut u8) -> bool {
    let header = unsafe { crate::header(block) };
    header.type_id != TYPE_ID_INVALID && (header.is_marked() || header.is_immortal())
}

fn block_size(block: *mut u8) -> usize {
    unsafe { crate::header(block) }.size as usize
}

fn write_block_size(block: *mut u8, size: usize) {
    unsafe { crate::header(block) }.size = size as u64;
}

fn write_free_header(block: *mut u8, size: usize) {
    let header = unsafe { crate::header(block) };
    header.type_id = TYPE_ID_INVALID;
    header.flags = 0;
    header.size = size as u64;
}

static HEAP: Global<Heap> = Global::new(Heap::new());

pub(crate) fn heap() -> &'static mut Heap {
    HEAP.get()
}

// ===== duong di cua mot lan cap phat =====
