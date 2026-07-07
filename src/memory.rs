use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    PROCESSENTRY32W,
};
use windows::Win32::System::Memory::{
    VirtualQueryEx, MEMORY_BASIC_INFORMATION, MEM_COMMIT, MEM_PRIVATE, PAGE_GUARD,
    PAGE_NOACCESS, PAGE_READONLY, PAGE_READWRITE, PAGE_WRITECOPY, PAGE_EXECUTE_READ,
    PAGE_EXECUTE_READWRITE, PAGE_EXECUTE_WRITECOPY, PAGE_PROTECTION_FLAGS,
};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ};

use indicatif::ProgressBar;

const CHUNK_SIZE: usize = 1024 * 1024; // 1 MB per read

pub struct ProcessHandle {
    handle: HANDLE,
}

fn find_process_id(name: &str) -> Option<u32> {
    unsafe {
        let snapshot: HANDLE = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            Ok(s) => s,
            Err(_) => return None,
        };
        let mut entry = PROCESSENTRY32W::default();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        if Process32FirstW(snapshot, &mut entry).is_err() {
            let _ = CloseHandle(snapshot);
            return None;
        }

        let target_wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();

        loop {
            let exe_matches = entry.szExeFile[..target_wide.len()] == *target_wide.as_slice();
            if exe_matches {
                let pid = entry.th32ProcessID;
                let _ = CloseHandle(snapshot);
                return Some(pid);
            }

            if Process32NextW(snapshot, &mut entry).is_err() {
                break;
            }
        }

        let _ = CloseHandle(snapshot);
        None
    }
}

unsafe impl Sync for ProcessHandle {}

impl ProcessHandle {
    pub fn open(name: &str) -> Option<Self> {
        let pid = find_process_id(name)?;
        println!("Connected to MTGA (PID: {})", pid);

        let handle = unsafe {
            OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid).ok()
        }?;

        Some(ProcessHandle { handle })
    }

    pub fn read_bytes(&self, address: u64, size: usize) -> Option<Vec<u8>> {
        if size == 0 {
            return Some(Vec::new());
        }
        let mut buf = vec![0u8; size];
        let mut bytes_read: usize = 0;
        let result = unsafe {
            ReadProcessMemory(
                self.handle,
                address as *const std::ffi::c_void,
                buf.as_mut_ptr() as *mut std::ffi::c_void,
                size,
                Some(&mut bytes_read),
            )
        };
        if result.is_ok() && bytes_read == size {
            Some(buf)
        } else {
            None
        }
    }

    pub fn list_readable_regions(&self) -> Vec<(u64, usize)> {
        let mut infos = Vec::new();
        let mut addr: u64 = 0;

        loop {
            let mut mbi = MEMORY_BASIC_INFORMATION::default();
            let result = unsafe {
                VirtualQueryEx(
                    self.handle,
                    Some(addr as *const std::ffi::c_void),
                    &mut mbi,
                    std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
                )
            };
            if result == 0 {
                break;
            }
            let region_size = mbi.RegionSize;
            if region_size == 0 {
                break;
            }

            let state = mbi.State;
            let protect = mbi.Protect;
            let is_readable = state == MEM_COMMIT
                && mbi.Type == MEM_PRIVATE
                && (protect & PAGE_NOACCESS) == PAGE_PROTECTION_FLAGS(0)
                && (protect & PAGE_GUARD) == PAGE_PROTECTION_FLAGS(0)
                && (protect
                    & (PAGE_READONLY
                        | PAGE_READWRITE
                        | PAGE_WRITECOPY
                        | PAGE_EXECUTE_READ
                        | PAGE_EXECUTE_READWRITE
                        | PAGE_EXECUTE_WRITECOPY))
                    != PAGE_PROTECTION_FLAGS(0);

            if is_readable {
                infos.push((mbi.BaseAddress as u64, region_size as usize));
            }

            addr = mbi.BaseAddress as u64 + region_size as u64;
            if addr == 0 {
                break;
            }
        }

        infos
    }
}

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.handle) };
    }
}

pub struct MemorySource {
    handle: ProcessHandle,
    pub region_infos: Vec<(u64, usize)>,
}

impl MemorySource {
    pub fn from_process() -> Option<Self> {
        let handle = ProcessHandle::open("MTGA.exe")?;
        let region_infos = handle.list_readable_regions();
        let total_size: usize = region_infos.iter().map(|&(_, s)| s).sum();
        println!(
            "Found {} readable regions ({} MB total).",
            region_infos.len(),
            total_size / 1024 / 1024
        );
        Some(MemorySource {
            handle,
            region_infos,
        })
    }

    pub fn pattern_scan(&self, pattern: &[u8], pb: &ProgressBar) -> Vec<u64> {
        if pattern.is_empty() {
            return Vec::new();
        }

        let total = self.region_infos.len();
        let done = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));

        let updater_done = done.clone();
        let updater_stop = stop.clone();
        let updater_pb = pb.clone();
        let handle = std::thread::spawn(move || loop {
            if updater_stop.load(Ordering::Relaxed) {
                break;
            }
            let d = updater_done.load(Ordering::Relaxed);
            updater_pb.set_position(d as u64);
            if d >= total {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        });

        use rayon::prelude::*;

        let pattern_vec = pattern.to_vec();
        let results: Vec<Vec<u64>> = self
            .region_infos
            .par_iter()
            .map(|&(base, size)| {
                let mut addrs = Vec::new();
                let mut offset = 0usize;
                while offset < size {
                    let read_size = CHUNK_SIZE.min(size - offset);
                    if let Some(data) = self.handle.read_bytes(base + offset as u64, read_size) {
                        if data.len() >= pattern_vec.len() {
                            let mut pos = 0;
                            while pos + pattern_vec.len() <= data.len() {
                                if data[pos..pos + pattern_vec.len()] == pattern_vec {
                                    addrs.push(base + offset as u64 + pos as u64);
                                }
                                pos += 1;
                            }
                        }
                    }
                    offset += CHUNK_SIZE;
                }
                done.fetch_add(1, Ordering::Relaxed);
                addrs
            })
            .collect();

        stop.store(true, Ordering::Relaxed);
        let _ = handle.join();
        pb.set_position(total as u64);

        results.into_iter().flatten().collect()
    }

    pub fn find_blocks(
        &self,
        addr: u64,
        offset_back: usize,
        read_size: usize,
    ) -> Vec<HashMap<u32, u32>> {
        let read_start = if addr >= offset_back as u64 {
            addr - offset_back as u64
        } else {
            0
        };

        if let Some(data) = self.handle.read_bytes(read_start, read_size) {
            parse_blocks(&data)
        } else {
            Vec::new()
        }
    }
}

fn parse_blocks(data: &[u8]) -> Vec<HashMap<u32, u32>> {
    if data.len() < 8 {
        return Vec::new();
    }

    let ints: &[u32] = bytemuck::cast_slice(data);
    let mut blocks = Vec::new();

    for off in 0..=1 {
        let mut curr = HashMap::new();
        let mut misses = 0;

        let mut i = off;
        while i + 1 < ints.len() {
            let k = ints[i];
            let v = ints[i + 1];

            if (1000..500000).contains(&k) && (1..=400).contains(&v) {
                curr.insert(k, v);
                misses = 0;
            } else {
                misses += 1;
            }

            if misses > 50 {
                if curr.len() > 50 {
                    blocks.push(std::mem::take(&mut curr));
                }
                curr.clear();
                misses = 0;
            }

            i += 2;
        }

        if curr.len() > 50 {
            blocks.push(curr);
        }
    }

    blocks
}
