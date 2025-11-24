# Trust-T

**High-performance Rust concurrency primitives using trust-based isolation** — achieve atomic-like performance with exclusive access guarantees through dedicated worker threads and zero-copy SPSC channels.

## Features

- **Local Trust**: Single-threaded execution with zero allocation and no atomics
- **Remote Trust**: Cross-thread execution via lock-free SPSC channels and dedicated worker threads
- **Fiber Pooling**: Optimized `launch` API with fiber reuse for low latency
- **Local Shortcut**: Automatic detection and bypass of channel overhead when calling from the worker thread

## Quick Start

```rust
use trust_tee::prelude::*;

// Local trust - single-threaded, zero overhead
let local = Local::entrust(42u32);
let result = local.apply(|x| *x + 1);

// Remote trust - cross-thread with dedicated worker
let remote = Remote::entrust(200u32);
remote.apply(|x| *x += 1);
let result = remote.with(|x| *x as u64);

// Batch operations for better throughput
remote.apply_batch_mut(|x| *x += 1, 16);

// Launch concurrent fiber operations
let value = remote.launch(|x| expensive_computation(x));
```

## Architecture

### Module Structure

- `trust/` - Core trust trait and implementations (`Local`, `Remote`)
- `runtime/` - Remote runtime with worker thread, slot-based protocol, and fiber management
- `single_thread/` - Single-threaded primitives (Latch)
- `util/` - Utilities (wait budgets, affinity, Miri-compatible fibers)

### Key Types

- `Local<T>` - Local trust for single-threaded execution
- `Remote<T>` - Remote trust using SPSC channels and worker thread
- `Runtime<T>` - Worker thread runtime managing property and client requests
- `Latch<T>` - Single-threaded mutual exclusion without atomics

## Cargo Features

- `default` - Local and remote trust functionality

## Testing

```bash
cargo test

MIRIFLAGS="-Zmiri-many-seeds -Zdeduplicate-diagnostics -Zmiri-strict-provenance" cargo miri test

cargo bench --bench contention
```

## Examples

- `examples/remote_simple.rs` - Basic remote trust usage
- `examples/remote_complex.rs` - Complex multi-client scenarios
- `examples/recursive_apply.rs` - Local shortcut demonstration
- `examples/launch_test.rs` - Fiber pooling and concurrent operations

## References

This implementation is based (as closely as possible) on the paper:

> "Delegation with Trust<T>: A Scalable, Type- and Memory-Safe Alternative to Locks"

The design prioritizes the no-atomics approach described in the paper, using SPSC channels and trustee-side execution to achieve high performance without atomic instructions.

Arxiv: [Delegation with Trust<T>: A Scalable, Type- and Memory-Safe Alternative to Locks](https://arxiv.org/abs/2408.11173)

## License

This project is licensed under the MIT License. See the LICENSE file for details.
