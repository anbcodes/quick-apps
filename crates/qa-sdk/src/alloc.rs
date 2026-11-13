//! A small allocator: power-of-two size classes with free lists, plus a
//! first-fit list for large blocks. Memory only grows; freed blocks are reused.

use core::alloc::{GlobalAlloc, Layout};

const CLASSES: usize = 12; // 16 B .. 32 KiB
const PAGE: usize = 65536;

pub struct Alloc;

static mut FREE: [usize; CLASSES] = [0; CLASSES];
/// Large free blocks: [size, next] headers.
static mut LARGE: usize = 0;
static mut BUMP: usize = 0;
static mut END: usize = 0;

extern "C" {
    static __heap_base: u8;
}

unsafe fn bump(size: usize, align: usize) -> *mut u8 {
    if BUMP == 0 {
        BUMP = core::ptr::addr_of!(__heap_base) as usize;
        END = core::arch::wasm32::memory_size(0) * PAGE;
    }
    let start = (BUMP + align - 1) & !(align - 1);
    let end = start + size;
    if end > END {
        let pages = (end - END).div_ceil(PAGE);
        if core::arch::wasm32::memory_grow(0, pages) == usize::MAX {
            return core::ptr::null_mut();
        }
        END += pages * PAGE;
    }
    BUMP = end;
    start as *mut u8
}

fn class(size: usize) -> usize {
    let s = size.max(16);
    (usize::BITS - (s - 1).leading_zeros()) as usize - 4
}

unsafe impl GlobalAlloc for Alloc {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let align = l.align().max(16);
        let c = class(l.size().max(align));
        if c < CLASSES && align == 16 {
            let head = FREE[c];
            if head != 0 {
                FREE[c] = *(head as *const usize);
                return head as *mut u8;
            }
            return bump(16 << c, 16);
        }
        let size = (l.size() + 15) & !15;
        if align == 16 {
            // First fit among freed large blocks.
            let mut prev: *mut usize = core::ptr::addr_of_mut!(LARGE);
            let mut cur = LARGE;
            while cur != 0 {
                let bsize = *(cur as *const usize);
                let next = *((cur + 4) as *const usize);
                if bsize >= size {
                    *prev = next;
                    return cur as *mut u8;
                }
                prev = (cur + 4) as *mut usize;
                cur = next;
            }
        }
        bump(size, align)
    }

    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        let align = l.align().max(16);
        let c = class(l.size().max(align));
        if c < CLASSES && align == 16 {
            *(p as *mut usize) = FREE[c];
            FREE[c] = p as usize;
        } else if align == 16 {
            *(p as *mut usize) = (l.size() + 15) & !15;
            *((p as usize + 4) as *mut usize) = LARGE;
            LARGE = p as usize;
        }
    }
}
