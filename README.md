# Trust-T

A Rust library providing trust-based concurrency primitives for local and remote execution.

## Features

- **Local Trust**: Single-threaded execution with zero allocation and no atomics
- **Remote Trust**: Cross-thread execution via SPSC queues and worker threads
- **Latch**: Single-threaded mutual exclusion without atomic instructions
- **Feature-gated**: Remote functionality behind `remote` feature flag

## Quick Start

```rust
use trust_tee::prelude::*;

// Local trust - single-threaded
let local = Local::new(42u32);
let result = local.with_mut(|x| *x + 1);

// With Latch for mutual exclusion
let latch_trust = Local::new(Latch::new(100u32));
let result = latch_trust.lock_apply(|x| *x + 10);

#[cfg(feature = "remote")]
{
    // Remote trust - cross-thread
    let (runtime, remote) = Runtime::spawn(200u32, 1024);
    remote.apply_mut(|x| *x += 1);
    let result = remote.map_u64(|x| *x as u64);
}
```

## Architecture

### Module Structure

- `trust/` - Local and remote trust implementations
- `single_thread/` - Single-threaded primitives (Latch, launch APIs)
- `runtime/` - Remote runtime for cross-thread execution
- `util/` - Shared utilities (waiting, affinity, fiber management)

### Key Types

- `Local<T>` - Local trust for single-threaded execution
- `Remote<T>` - Remote trust for cross-thread execution (requires `remote` feature)
- `Latch<T>` - Single-threaded mutual exclusion without atomics
- `Runtime<T>` - Remote runtime manager (requires `remote` feature)

### Safety Guarantees

- Local path: Uses `UnsafeCell` with fiber scope guards
- Remote path: Uses function pointers for cross-thread safety
- Latch: Single-threaded only, panics on re-entrance
- Launch APIs: Only available for `Local<Latch<T>>` to prevent races

## Cargo Features

- `local` (default) - Local trust functionality
- `remote` - Remote trust and runtime functionality

## Examples

See `examples/basic_usage.rs` for a complete example.
