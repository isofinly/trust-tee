#![allow(missing_docs)]
use core::{mem::align_of, mem::size_of};

use std::ptr;
use std::sync::OnceLock;

use crate::runtime::slots::{
    Aligned, ErasedVTable, FatClosurePtr, RequestRecordHeader, SLOT_BYTES,
};

impl<Args, Ret> ErasedVTable<Args, Ret> {
    /// Returns the monomorphized erased vtable for a specific closure type `F`.
    ///
    /// - One singleton is created per `{Args, Ret, F}` instantiation.
    /// - The vtable provides `call_once_in_place` (move `F` out of the slot and run)
    ///   and `drop_in_place` (drop the captured `F` in the slot without calling it).
    /// - For zero-sized `F`, calling does not read from memory and constructs a fresh ZST.
    ///
    /// Strict provenance: `env` given to vtable functions must be derived from the
    /// original allocation (e.g., slot buffer) using in-bounds pointer arithmetic.
    ///
    /// Type parameters:
    /// - `Args`: argument tuple the closure accepts.
    /// - `Ret`: return type of the closure.
    /// - `F`: concrete closure type implementing `FnOnce(Args) -> Ret + Send + 'static`.
    pub fn for_closure<F>() -> &'static Self
    where
        F: FnOnce(Args) -> Ret + Send + 'static,
    {
        #[allow(non_camel_case_types)]
        struct PER_F<Args, Ret, F>(core::marker::PhantomData<(Args, Ret, F)>);
        impl<Args, Ret, F> PER_F<Args, Ret, F> {
            fn cell() -> &'static OnceLock<ErasedVTable<Args, Ret>> {
                static CELL: OnceLock<ErasedVTable<(), ()>> = OnceLock::new();
                // SAFETY: Each monomorphization gets a distinct CELL.
                unsafe { &*(&CELL as *const _ as *const OnceLock<ErasedVTable<Args, Ret>>) }
            }
        }

        PER_F::<Args, Ret, F>::cell().get_or_init(|| ErasedVTable {
            call_once_in_place: |env, args| {
                // For ZST captures there is no stored instance; construct a fresh ZST.
                // For non-ZST, move F out of slot memory, then call it.
                unsafe {
                    if size_of::<F>() == 0 {
                        let f: F = core::mem::MaybeUninit::<F>::uninit().assume_init();
                        f(args)
                    } else {
                        let f_ptr = env.cast::<F>();
                        let f: F = ptr::read(f_ptr); // move, not copy
                        f(args)
                        // f is dropped here; any captured resources are released.
                    }
                }
            },
            drop_in_place: |env| unsafe {
                if size_of::<F>() != 0 {
                    ptr::drop_in_place::<F>(env.cast::<F>());
                }
            },
            env_align: align_of::<F>() as u32,
            env_size: size_of::<F>() as u32,
        })
    }
}

pub struct SlotWriter {
    pub base: *mut u8,
    pub len: usize,
    pub cursor: usize,
}

impl SlotWriter {
    /// Creates a writer for a fixed-size, properly aligned slot buffer.
    ///
    /// - The writer holds a raw base pointer and length;
    /// - The internal `cursor` starts at 0.
    ///
    /// Safety model: All writes are performed via `base.add(offset)` and bounds-checked
    /// against `len` to maintain provenance and prevent overflow.
    ///
    /// Parameters:
    /// - `buf`: an `Aligned<SLOT_BYTES>` buffer that serves as the backing slot storage.
    ///
    /// Returns: a `SlotWriter` positioned at the beginning of `buf`.
    ///
    /// Example:
    /// ```
    /// use trust_tee::runtime::{serialization::SlotWriter, slots::{Aligned, SLOT_BYTES}};
    ///
    /// let mut storage = Aligned([0u8; SLOT_BYTES]);
    /// let mut writer = SlotWriter::new(&mut storage);
    ///
    /// assert_eq!(writer.cursor, 0);
    /// ```
    pub fn new(buf: &mut Aligned<SLOT_BYTES>) -> Self {
        Self {
            base: buf.0.as_mut_ptr(),
            len: buf.0.len(),
            cursor: 0,
        }
    }

    /// Reserves `size` bytes at the next `align` boundary and returns a raw pointer
    /// into the slot. Advances the internal cursor.
    ///
    /// Panics if the reservation would exceed the slot capacity.
    ///
    /// Strict provenance: The returned pointer is derived from the original base
    /// pointer with in-bounds arithmetic only.
    ///
    /// Parameters:
    /// - `size`: number of bytes to reserve.
    /// - `align`: required alignment (power-of-two). Alignment of 0 is treated as 1.
    ///
    /// Returns: a `*mut u8` pointing to the start of the reserved region.
    pub fn alloc_aligned(&mut self, size: usize, align: usize) -> *mut u8 {
        let base = self.base;
        let start = base.wrapping_add(self.cursor);
        let off = start.align_offset(align.max(1));
        let pos = self.cursor + off;
        self.cursor = pos + size;
        assert!(self.cursor <= self.len, "slot overflow");
        unsafe { base.add(pos) }
    }

    /// Writes the given `bytes` at the current cursor (no additional alignment)
    /// and returns a raw pointer to the written region. Advances the cursor.
    ///
    /// Panics if the write would exceed the slot capacity.
    ///
    /// Parameters:
    /// - `bytes`: slice to copy into the slot.
    ///
    /// Returns: a `*mut u8` pointing to the start of the written region.
    pub fn write_bytes(&mut self, bytes: &[u8]) -> *mut u8 {
        let p = self.alloc_aligned(bytes.len(), 1);
        unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len()) }
        p
    }
}

/// Encodes `f: F` and its serialized argument bytes into the slot, returning a `RequestRecordHeader`.
///
/// Layout written into the slot (starting at the writer's current `cursor`):
/// - Capture env of `F` (size `vt.env_size`, alignment `vt.env_align`)
/// - Serialized args (byte-aligned, immediately following env)
///
/// The returned header contains an erased fat pointer to the closure env and the
/// vtable, plus metadata lengths for env and args.
///
/// Strict provenance: All pointers are derived from the writer's base pointer via
/// in-bounds arithmetic; no integer-to-pointer roundtrips are performed.
///
/// Safety: This function is safe to call, but the resulting header must be used
/// only while the slot memory is still allocated and unchanged. Executing or
/// dropping the capture is performed by `decode_and_call` or `vt.drop_in_place`.
///
/// Parameters:
/// - `out`: destination `SlotWriter` into which the env and args are written.
/// - `property_ptr`: opaque 64-bit field carried through the `RequestRecordHeader`.
///   It is not dereferenced by this module; it exists so the producer can pass
///   a routing key, object handle, capability address, or any other metadata to
///   the consumer without additional lookups. The trustee can interpret it in
///   any domain-specific way.
/// - `f`: the `FnOnce(Args) -> Ret + Send + 'static` closure to encode.
/// - `serialized_args`: argument bytes placed immediately after the capture env.
///
/// Returns: a `RequestRecordHeader` describing the encoded closure and argument bytes.
///
/// Example:
/// ```
/// use trust_tee::runtime::serialization::{encode_closure, decode_and_call, SlotWriter};
/// use trust_tee::runtime::slots::{Aligned, SLOT_BYTES};
///
/// let mut storage = Aligned([0u8; SLOT_BYTES]);
/// let mut out = SlotWriter::new(&mut storage);
///
/// let header = encode_closure::<_, (), String>(&mut out, 0, |()| "ok".to_string(), &[]);
/// let result: String = unsafe { decode_and_call::<(), String>(&header, ()) };
///
/// assert_eq!(result, "ok");
/// ```
///
/// Examples of different callable forms (each in isolation):
///
/// fn item (coerces to Fn/FnMut/FnOnce):
/// ```
/// use trust_tee::runtime::serialization::{encode_closure, decode_and_call, SlotWriter};
/// use trust_tee::runtime::slots::{Aligned, SLOT_BYTES};
///
/// let mut storage = Aligned([0u8; SLOT_BYTES]);
/// let mut out = SlotWriter::new(&mut storage);
///
/// fn f0(_: ()) -> u32 { 7 }
///
/// let h_fn = encode_closure::<_, (), u32>(&mut out, 0, f0, &[]);
/// let r_fn: u32 = unsafe { decode_and_call::<(), u32>(&h_fn, ()) };
///
/// assert_eq!(r_fn, 7);
/// ```
///
/// Fn closure (shared/ZST capture):
/// ```
/// use trust_tee::runtime::serialization::{encode_closure, decode_and_call, SlotWriter};
/// use trust_tee::runtime::slots::{Aligned, SLOT_BYTES};
///
/// let mut storage = Aligned([0u8; SLOT_BYTES]);
/// let mut out = SlotWriter::new(&mut storage);
///
/// let add = |(a, b): (i32, i32)| a + b;
///
/// let h_fn_like = encode_closure::<_, (i32, i32), i32>(&mut out, 0, add, &[]);
/// let r_fn_like: i32 = unsafe { decode_and_call::<(i32, i32), i32>(&h_fn_like, (2, 3)) };
///
/// assert_eq!(r_fn_like, 5);
/// ```
///
/// FnMut closure (mutates capture), invoked via FnOnce path:
/// ```
/// use trust_tee::runtime::serialization::{encode_closure, decode_and_call, SlotWriter};
/// use trust_tee::runtime::slots::{Aligned, SLOT_BYTES};
///
/// let mut storage = Aligned([0u8; SLOT_BYTES]);
/// let mut out = SlotWriter::new(&mut storage);
///
/// let mut acc = 10i32;
/// let mut c = move |x: i32| { acc += x; acc };
///
/// let h_fnmut = encode_closure::<_, i32, i32>(&mut out, 0, move |x| c(x), &[]);
/// let r_fnmut: i32 = unsafe { decode_and_call::<i32, i32>(&h_fnmut, 5) };
///
/// assert_eq!(r_fnmut, 15);
/// ```
///
/// FnOnce closure (consumes capture):
/// ```
/// use trust_tee::runtime::serialization::{encode_closure, decode_and_call, SlotWriter};
/// use trust_tee::runtime::slots::{Aligned, SLOT_BYTES};
///
/// let mut storage = Aligned([0u8; SLOT_BYTES]);
/// let mut out = SlotWriter::new(&mut storage);
///
/// let s = String::from("hi");
///
/// let h_fnonce = encode_closure::<_, (), String>(&mut out, 0, move |()| s + "!", &[]);
/// let r_fnonce: String = unsafe { decode_and_call::<(), String>(&h_fnonce, ()) };
///
/// assert_eq!(r_fnonce, "hi!");
/// ```
pub fn encode_closure<F, Args: 'static, Ret: 'static>(
    out: &mut SlotWriter,
    property_ptr: u64,
    f: F,
    serialized_args: &[u8],
) -> RequestRecordHeader
where
    F: FnOnce(Args) -> Ret + Send + 'static,
{
    let env_size = size_of::<F>();
    let env_align = align_of::<F>();

    // Build vtable locally and place it inside the slot for this record.
    let (env_ptr, args_ptr_in_slot, vt_ptr_any) = unsafe {
        // Base pointer and current cursor
        let base = out.base;
        let len = out.len;
        let cursor0 = out.cursor;

        // 1) Allocate space for vtable
        let vt_align = align_of::<ErasedVTable<Args, Ret>>();
        let start_vt = base.wrapping_add(cursor0);
        let off_vt = start_vt.align_offset(vt_align.max(1));
        let vt_pos = cursor0 + off_vt;
        let after_vt = vt_pos + size_of::<ErasedVTable<Args, Ret>>();
        assert!(after_vt <= len, "slot overflow");
        let vt_ptr_in_slot = base.add(vt_pos) as *mut ErasedVTable<Args, Ret>;

        // Initialize vtable for F
        let vt_value = ErasedVTable::<Args, Ret> {
            call_once_in_place: |env, args| {
                // For ZST captures construct a fresh instance; otherwise move from slot
                if size_of::<F>() == 0 {
                    let f: F = core::mem::MaybeUninit::<F>::uninit().assume_init();
                    f(args)
                } else {
                    let f_ptr = env.cast::<F>();
                    let f: F = ptr::read(f_ptr);
                    f(args)
                }
            },
            drop_in_place: |env| {
                if size_of::<F>() != 0 {
                    ptr::drop_in_place::<F>(env.cast::<F>());
                }
            },
            env_align: env_align as u32,
            env_size: env_size as u32,
        };
        ptr::write(vt_ptr_in_slot, vt_value);

        // 2) Allocate env with alignment
        let start_env = base.wrapping_add(after_vt);
        let off_env = start_env.align_offset(env_align.max(1));
        let env_pos = after_vt + off_env;
        let after_env = env_pos + env_size;
        assert!(after_env <= len, "slot overflow");
        let env_ptr_in_slot = base.add(env_pos);

        // Write F into env (if non-ZST)
        if env_size != 0 {
            ptr::write((env_ptr_in_slot as *mut u8).cast::<F>(), f);
        }

        // 3) Args are written immediately after env; args alignment is 1
        let args_pos = after_env;
        let after_args = args_pos + serialized_args.len();
        assert!(after_args <= len, "slot overflow");
        let args_ptr_in_slot = base.add(args_pos);
        ptr::copy_nonoverlapping(
            serialized_args.as_ptr(),
            args_ptr_in_slot,
            serialized_args.len(),
        );

        // Advance cursor once at the end
        out.cursor = after_args;

        (
            if env_size == 0 {
                core::ptr::NonNull::<u8>::dangling().as_ptr()
            } else {
                env_ptr_in_slot as *mut u8
            },
            args_ptr_in_slot,
            vt_ptr_in_slot as *const (),
        )
    };

    RequestRecordHeader {
        closure: FatClosurePtr {
            data: env_ptr,
            vtable: vt_ptr_any,
        },
        property_ptr,
        captured_len: env_size as u32,
        args_len: serialized_args.len() as u32,
        args_offset: (args_ptr_in_slot as usize - (out.base as usize)) as u32,
    }
}

/// Decodes the header and invokes the encoded `FnOnce` with the provided `args`.
/// This moves `F` out of the slot (for non-ZST captures) and runs it exactly once.
///
/// Returns the closure's result.
///
/// Safety:
/// - `hdr` must have been produced by `encode_closure` into still-live, writable
///   memory; its env pointer must be valid for a `ptr::read` of type `F` and for
///   `drop_in_place` if not executed.
/// - The `Args` used here must match what the closure expects and how the caller
///   deserialized them from the bytes written after the env.
/// - No concurrent aliasing writes/read-writes may happen to the env region.
///
/// Parameters:
/// - `hdr`: header returned from `encode_closure`.
/// - `args`: argument tuple to pass to the closure (caller deserialized it).
///
/// Example:
/// ```
/// use trust_tee::runtime::serialization::{encode_closure, decode_and_call, SlotWriter};
/// use trust_tee::runtime::slots::{Aligned, SLOT_BYTES};
///
/// let mut storage = Aligned([0u8; SLOT_BYTES]);
/// let mut out = SlotWriter::new(&mut storage);
///
/// let hdr = encode_closure::<_, (u32,), u32>(&mut out, 0, |(x,)| x + 1, &[]);
/// let res: u32 = unsafe { decode_and_call::<(u32,), u32>(&hdr, (41,)) };
///
/// assert_eq!(res, 42);
/// ```
///
/// Deserializing args bytes (before calling):
/// ```
/// use trust_tee::runtime::serialization::{encode_closure, decode_and_call, SlotWriter};
/// use trust_tee::runtime::slots::{Aligned, SLOT_BYTES, RequestRecordHeader};
/// let mut storage = Aligned([0u8; SLOT_BYTES]);
/// let mut out = SlotWriter::new(&mut storage);
/// // Producer encodes raw arg bytes; here we serialize (u32, u32) as little-endian.
/// let args = (1u32, 2u32);
/// let mut tmp = [0u8; 8];
/// tmp[0..4].copy_from_slice(&args.0.to_le_bytes());
/// tmp[4..8].copy_from_slice(&args.1.to_le_bytes());
/// // Remember record start so trustee can locate args relative to slot base.
/// let record_start = out.cursor;
/// let hdr = encode_closure::<_, (u32, u32), u32>(&mut out, 0, |(a,b)| a + b, &tmp);
///
/// // Trustee computes the args slice: immediately after the capture env.
/// // Use slot base + record_start to be robust even for ZST captures (env pointer may be dangling).
/// unsafe {
///     let base = out.base;
///     let args_ptr = base.add(record_start + hdr.captured_len as usize);
///     let bytes = core::slice::from_raw_parts(args_ptr, hdr.args_len as usize);
///     // Deserialize back to (u32, u32)
///     let a = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
///     let b = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
///     let sum: u32 = decode_and_call::<(u32, u32), u32>(&hdr, (a, b));
///     assert_eq!(sum, 3);
/// }
/// ```
pub unsafe fn decode_and_call<Args, Ret>(
    hdr: &RequestRecordHeader,
    // Caller deserializes Args from the bytes that follow the capture.
    args: Args,
) -> Ret {
    unsafe {
        let vt = &*(hdr.closure.vtable as *const ErasedVTable<Args, Ret>);
        let env_ptr = hdr.closure.data;

        // If the trustee decides not to execute, it must drop the capture:
        // (vt.drop_in_place)(env_ptr);

        // Normal path: execute once; this moves F out of slot and runs it.
        (vt.call_once_in_place)(env_ptr, args)
    }
}
