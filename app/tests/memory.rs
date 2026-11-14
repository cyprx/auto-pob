//! malloc's fixed mmap threshold (engine::memory::fix_mmap_threshold), in a process of its own:
//! the threshold is process-wide.

#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[test]
fn a_big_buffer_stays_mapped_after_a_bigger_one_was_freed() {
    #[repr(C)]
    struct Mallinfo2 {
        arena: usize,
        ordblks: usize,
        smblks: usize,
        hblks: usize,
        hblkhd: usize,
        usmblks: usize,
        fsmblks: usize,
        uordblks: usize,
        fordblks: usize,
        keepcost: usize,
    }
    unsafe extern "C" {
        fn mallinfo2() -> Mallinfo2;
    }
    // SAFETY: mallinfo2 has no preconditions
    let mapped = || unsafe { mallinfo2() }.hblks;
    assert!(auto_pob::engine::memory::fix_mmap_threshold());
    // freeing a big mapped block is what makes glibc raise a dynamic threshold to its size
    drop(std::hint::black_box(vec![1u8; 16 << 20]));
    let before = mapped();
    // a decoded file's worth: still its own mapping, which free hands back
    let held = std::hint::black_box(vec![1u8; 4 << 20]);
    assert_eq!(mapped(), before + 1);
    drop(held);
    assert_eq!(mapped(), before);
}
