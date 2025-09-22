//! Cross-platform thread pinning and optional NUMA memory policy.

#[derive(Clone, Copy, Debug, Default)]
pub struct PinConfig {
    pub core_id: Option<usize>,        // Linux
    pub numa_node: Option<u16>,        // Linux
    pub mem_bind: bool,                // Linux
    pub mac_affinity_tag: Option<i32>, // macOS
}

pub fn pin_current_thread(cfg: &PinConfig) {
    #[cfg(target_os = "linux")]
    {
        use core::mem::{size_of, zeroed};
        unsafe {
            if let Some(core) = cfg.core_id {
                let mut set: libc::cpu_set_t = zeroed();
                let bits_ptr = &mut set as *mut _ as *mut u64;
                let idx = core / 64;
                let off = core % 64;
                *bits_ptr.add(idx) |= 1u64 << off;
                let pthread = libc::pthread_self();
                let _ = libc::pthread_setaffinity_np(
                    pthread,
                    size_of::<libc::cpu_set_t>(),
                    &set as *const libc::cpu_set_t,
                );
            }
            if let (Some(node), true) = (cfg.numa_node, cfg.mem_bind) {
                const MPOL_BIND: i32 = 2;
                let mut mask: u64 = 0;
                if (node as usize) < 64 {
                    mask |= 1u64 << (node as u32);
                    let _ = libc::syscall(
                        libc::SYS_set_mempolicy,
                        MPOL_BIND as libc::c_long,
                        &mask as *const u64,
                        64usize as libc::c_long,
                    );
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        unsafe {
            if let Some(tag) = cfg.mac_affinity_tag {
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
                let _ = thread_policy_set(
                    self_thread,
                    THREAD_AFFINITY_POLICY,
                    (&pol as *const thread_affinity_policy_data_t) as *const libc::integer_t,
                    cnt,
                );
            }
        }
    }
}
