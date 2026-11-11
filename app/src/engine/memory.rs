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

/// The resident set size, in bytes, where the platform reports it.
pub fn resident_bytes() -> Option<usize> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmRSS:"))?;
    let kb: usize = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb * 1024)
}
