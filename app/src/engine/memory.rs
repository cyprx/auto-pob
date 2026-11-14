//! Handing the memory malloc keeps after a busy stretch back to the OS.
//!
//! glibc keeps memory it has freed in its arenas instead of returning it to the OS. A node
//! power or gem DPS sort frees hundreds of megabytes that would otherwise stay in RSS.

/// Returns the free memory malloc holds to the OS.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub fn release_free_memory() {
    unsafe extern "C" {
        fn malloc_trim(pad: usize) -> i32;
    }

    // SAFETY: malloc_trim has no preconditions
    unsafe { malloc_trim(0) };
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub fn release_free_memory() {}

/// glibc's `M_MMAP_THRESHOLD` (malloc.h)
#[cfg(all(target_os = "linux", target_env = "gnu"))]
const M_MMAP_THRESHOLD: i32 = -3;

/// Allocations of this size and up get their own mapping, returned to the OS when freed
pub const MMAP_THRESHOLD: usize = 128 * 1024;

/// Fixes malloc's mmap threshold at [`MMAP_THRESHOLD`], before anything allocates much. glibc
/// otherwise raises the threshold to the size of the largest mapped block freed so far, after
/// which buffers up to that size (decoded tree art, up to ~28 MB) come from an arena's heap,
/// which neither free nor `malloc_trim` hands back: ~46 MB stayed resident after a build was
/// opened on the Tree tab. Returns whether malloc took it.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub fn fix_mmap_threshold() -> bool {
    unsafe extern "C" {
        fn mallopt(param: i32, value: i32) -> i32;
    }
    // SAFETY: mallopt has no preconditions
    unsafe { mallopt(M_MMAP_THRESHOLD, MMAP_THRESHOLD as i32) == 1 }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub fn fix_mmap_threshold() -> bool {
    true
}

/// The resident set size, in bytes, where the platform reports it.
pub fn resident_bytes() -> Option<usize> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmRSS:"))?;
    let kb: usize = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb * 1024)
}
