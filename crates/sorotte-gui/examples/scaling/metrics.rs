use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

pub struct CountingAllocator;
static CALLS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
static LIVE: AtomicU64 = AtomicU64::new(0);

// SAFETY: Every allocation is forwarded unchanged to System and each matching deallocation
// receives its original pointer/layout. Counters never allocate or affect returned storage.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The caller satisfies GlobalAlloc's layout requirements.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            CALLS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
            LIVE.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size() as u64, Ordering::Relaxed);
        // SAFETY: The original pointer and layout are passed through to its System allocator.
        unsafe { System.dealloc(pointer, layout) };
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: The caller provides a live System allocation and valid replacement size.
        let result = unsafe { System.realloc(pointer, layout, size) };
        if !result.is_null() {
            CALLS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(size as u64, Ordering::Relaxed);
            LIVE.fetch_add(size as u64, Ordering::Relaxed);
            LIVE.fetch_sub(layout.size() as u64, Ordering::Relaxed);
        }
        result
    }
}

#[derive(serde::Serialize)]
pub struct Measurement {
    pub nanoseconds: u64,
    pub allocation_calls: u64,
    pub allocated_bytes: u64,
    pub retained_bytes_delta: i128,
}

pub fn measure<T>(
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<(T, Measurement), String> {
    let calls = CALLS.load(Ordering::Relaxed);
    let bytes = BYTES.load(Ordering::Relaxed);
    let live = LIVE.load(Ordering::Relaxed);
    let started = Instant::now();
    let result = operation()?;
    Ok((
        result,
        Measurement {
            nanoseconds: started.elapsed().as_nanos() as u64,
            allocation_calls: CALLS.load(Ordering::Relaxed) - calls,
            allocated_bytes: BYTES.load(Ordering::Relaxed) - bytes,
            retained_bytes_delta: LIVE.load(Ordering::Relaxed) as i128 - live as i128,
        },
    ))
}

pub fn os_handles() -> Result<usize, String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessHandleCount};
        let mut count = 0;
        // SAFETY: The pseudo-handle refers to this live process, and count is a valid out pointer.
        if unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut count) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(count as usize)
    }
    #[cfg(target_os = "linux")]
    {
        std::fs::read_dir("/proc/self/fd")
            .map(|entries| entries.count())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Err("handle measurement supports Windows and Linux".to_owned())
    }
}
