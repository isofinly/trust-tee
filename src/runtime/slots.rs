#![allow(dead_code)]
#![allow(missing_docs)]
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU32};

#[repr(align(64))]
pub struct Aligned<const N: usize>(pub [u8; N]);

impl<const N: usize> Aligned<N> {
    pub const fn new(bytes: [u8; N]) -> Self {
        Self(bytes)
    }
}

/// Slot sizing (two-part optimization)
pub const PRIMARY_BYTES: usize = 128;
pub const TRAILING_BYTES: usize = 1024;
pub const SLOT_BYTES: usize = PRIMARY_BYTES + TRAILING_BYTES;
pub const HEADER_BYTES: usize = 4; // u32 flags at start of primary block

/// Header flags: bit0 is the ready/match bit; upper 31 bits carry request_count (request slot) or are reserved (response slot).
#[repr(transparent)]
pub struct SlotHeader {
    pub flags: AtomicU32,
}

// Helpers for encoding/decoding the header word.
pub mod header {
    pub const READY_BIT: u32 = 1 << 0;
    pub const COUNT_SHIFT: u32 = 1;
    pub const COUNT_MASK: u32 = 0xFFFF_FFFE; // bits [31:1]
    #[inline]
    pub const fn pack_ready_count(ready: bool, count: u32) -> u32 {
        ((count << COUNT_SHIFT) & COUNT_MASK) | (ready as u32)
    }
    #[inline]
    pub const fn ready(word: u32) -> bool {
        (word & READY_BIT) != 0
    }
    #[inline]
    pub const fn count(word: u32) -> u32 {
        (word & COUNT_MASK) >> COUNT_SHIFT
    }
}

/// A 128-bit fat pointer to a Rust closure: { data, vtable }.
#[repr(C)]
pub struct FatClosurePtr {
    pub data: *mut u8,     // where the capture lives inside the slot
    pub vtable: *const (), // erased pointer to ErasedVTable<Args, Ret>
}

#[repr(C)]
pub struct RequestRecordHeader {
    pub closure: FatClosurePtr,    // 16
    pub property_ptr: PropertyPtr, // 8
    pub captured_len: u32,         // env byte count
    pub args_len: u32,             // serialized args length
    pub args_offset: u32,          // offset from slot base to start of args bytes
                                   // trailing bytes: [captured_env][serialized_args]
}

#[repr(u64)]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum PropertyPtr {
    CallMutRetUnit = 0,
    CallMutOutPtr = 1,
    CallMutArgsOutPtr = 2,
    IntoInner = 3,
    Terminate = u64::MAX,
}

#[repr(C)]
pub struct ErasedVTable<Args, Ret> {
    pub call_once_in_place: unsafe fn(env: *mut u8, args: Args) -> Ret,
    pub drop_in_place: unsafe fn(env: *mut u8),
    pub env_align: u32,
    pub env_size: u32,
}

pub const MIN_REQUEST_RECORD_BYTES: usize = core::mem::size_of::<FatClosurePtr>() + 8; // 24

/// Response record prefix; present only for variable-sized responses.
/// For fixed-size responses, the producer/consumer know the size out-of-band,
/// and this prefix is omitted; zero-sized responses omit a payload entirely.
#[repr(C)]
pub struct VarResponsePrefix {
    pub len: u32, // length in bytes of the following response payload
                  // trailing bytes: [len]
}

/// Primary blocks place the header first, then a byte region that packs
/// one or more whole records (each record is fully in primary or fully in trailing).
#[repr(C)]
pub struct RequestPrimary {
    pub header: SlotHeader,                       // 4 bytes
    pub data: [u8; PRIMARY_BYTES - HEADER_BYTES], // packed RequestRecordHeader + payloads
}

#[repr(C)]
pub struct ResponsePrimary {
    pub header: SlotHeader, // bit0 mirrors/toggles to match a processed request
    pub data: [u8; PRIMARY_BYTES - HEADER_BYTES], // packed fixed or variable responses
}

/// Two-part request slot: primary then trailing; only the client writes this slot.
#[repr(C)]
pub struct RequestSlot {
    pub primary: RequestPrimary,
    pub trailing: [u8; TRAILING_BYTES], // packed whole RequestRecord(s) that do not fit in primary
}

/// Two-part response slot: primary then trailing; only the trustee writes this slot.
#[repr(C)]
pub struct ResponseSlot {
    pub primary: ResponsePrimary,
    pub trailing: [u8; TRAILING_BYTES], // packed whole response record(s); var-size responses are prefixed by VarResponsePrefix
}

/// One dedicated pair of slots per client/trustee pair.
/// Each send toggles primary.header bit0 and sets the request count in bits [31:1].
#[repr(C)]
pub struct ChannelPair {
    pub request: UnsafeCell<RequestSlot>,
    pub response: UnsafeCell<ResponseSlot>,
    // Client-side serialization to enforce SPSC on the request slot across clones.
    pub client_lock: AtomicBool,
}

impl Default for SlotHeader {
    fn default() -> Self {
        SlotHeader {
            flags: AtomicU32::new(0),
        }
    }
}

impl Default for RequestPrimary {
    fn default() -> Self {
        RequestPrimary {
            header: SlotHeader::default(),
            data: [0u8; PRIMARY_BYTES - HEADER_BYTES],
        }
    }
}

impl Default for ResponsePrimary {
    fn default() -> Self {
        ResponsePrimary {
            header: SlotHeader::default(),
            data: [0u8; PRIMARY_BYTES - HEADER_BYTES],
        }
    }
}

impl Default for RequestSlot {
    fn default() -> Self {
        RequestSlot {
            primary: RequestPrimary::default(),
            trailing: [0u8; TRAILING_BYTES],
        }
    }
}

impl Default for ResponseSlot {
    fn default() -> Self {
        ResponseSlot {
            primary: ResponsePrimary::default(),
            trailing: [0u8; TRAILING_BYTES],
        }
    }
}

impl Default for ChannelPair {
    fn default() -> Self {
        ChannelPair {
            request: UnsafeCell::new(RequestSlot::default()),
            response: UnsafeCell::new(ResponseSlot::default()),
            client_lock: AtomicBool::new(false),
        }
    }
}

// Safety: The request/response slots are mutated with explicit synchronization
// via atomic header flags and strict role separation (client writes request,
// trustee writes response). Interior mutability is modeled with UnsafeCell.
unsafe impl Sync for ChannelPair {}
unsafe impl Send for ChannelPair {}
