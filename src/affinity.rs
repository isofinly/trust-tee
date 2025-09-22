//! Cross-platform thread pinning and optional NUMA memory policy.
//!
//! Linux:
//!   - CPU pin: pthread_setaffinity_np on the current thread.
//!   - NUMA mem policy: set_mempolicy(MPOL_BIND, nodemask) best-effort.
//! macOS:
//!   - Affinity tag: thread_policy_set(THREAD_AFFINITY_POLICY).

/// Configuration for thread pinning and optional NUMA memory policy.
#[derive(Clone, Copy, Debug, Default)]
pub struct PinConfig {
    /// Optional logical core to pin the trustee OS thread to.
    pub core_id: Option<usize>,
    /// Optional NUMA node for memory binding (Linux only).
    pub numa_node: Option<u16>,
    /// Whether to apply MPOL_BIND when numa_node is set (Linux only).
    pub mem_bind: bool,
    /// Optional Mach thread affinity tag (macOS only).
    pub mac_affinity_tag: Option<i32>,
}

pub fn pin_current_thread(cfg: &PinConfig) {
    #[cfg(target_os = "linux")]
    {
        use core::mem::{size_of, zeroed};
        unsafe {
            if let Some(core) = cfg.core_id {
                let mut set: libc::cpu_set_t = zeroed();
                // cpu_set_t is a bitset; treat storage as u64 array.
                let bits_ptr = &mut set as *mut _ as *mut u64;
                let idx = core / 64;
                let off = core % 64;
                *bits_ptr.add(idx) |= 1u64 << off;
                let pthread = libc::pthread_self();
                let r = libc::pthread_setaffinity_np(
                    pthread,
                    size_of::<libc::cpu_set_t>(),
                    &set as *const libc::cpu_set_t,
                );
                let _ = r;
            }
            if let (Some(node), true) = (cfg.numa_node, cfg.mem_bind) {
                // set_mempolicy(MPOL_BIND=2, nodemask, maxnode)
                const MPOL_BIND: i32 = 2;
                let mut mask: u64 = 0;
                if (node as usize) < 64 {
                    mask |= 1u64 << (node as u32);
                    let _res = libc::syscall(
                        libc::SYS_set_mempolicy,
                        MPOL_BIND as libc::c_long,
                        &mask as *const u64,
                        64usize as libc::c_long,
                    );
                    let _ = _res;
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        // Use Mach thread affinity tags; core pinning is not supported directly.
        unsafe {
            if let Some(tag) = cfg.mac_affinity_tag {
                // Minimal subset of Mach APIs via libc.
                type thread_t = libc::mach_port_t;
                #[repr(C)]
                struct thread_affinity_policy_data_t {
                    affinity_tag: libc::integer_t,
                }
                unsafe extern "C" {
                    fn mach_thread_self() -> thread_t;
                    fn thread_policy_set(
                        thread: thread_t,
                        flavor: libc::c_int,
                        policy_info: *const libc::integer_t,
                        count: libc::mach_msg_type_number_t,
                    ) -> libc::kern_return_t;
                }
                const THREAD_AFFINITY_POLICY: libc::c_int = 4;
                let self_thread = mach_thread_self();
                let pol = thread_affinity_policy_data_t {
                    affinity_tag: tag as libc::integer_t,
                };
                let cnt = (core::mem::size_of::<thread_affinity_policy_data_t>()
                    / core::mem::size_of::<libc::integer_t>())
                    as libc::mach_msg_type_number_t;
                let _kr = thread_policy_set(
                    self_thread,
                    THREAD_AFFINITY_POLICY,
                    (&pol as *const thread_affinity_policy_data_t) as *const libc::integer_t,
                    cnt,
                );
                let _ = _kr;
            }
        }
    }
}
