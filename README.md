# Trust<T>: A Delegation-Based Alternative to Locking

Trust<T> is a Rust library that provides a scalable, type- and memory-safe alternative to locking for concurrent programs. Instead of synchronizing access to shared objects with locks, Trust<T> delegates all operations to designated trustee fibers through a message-passing protocol.

## Overview

Trust<T> is based on **delegation**, a message-passing technique that avoids the scalability limitations of traditional locking. Key benefits include:

- **No atomic operations**: All coordination is done through single-producer/single-consumer (SPSC) channels (WIP)
- **Higher throughput**: Per-object throughput is limited by trustee capacity rather than lock contention
- **Type safety**: Rust's type system ensures memory safety and prevents race conditions
- **Familiar API**: Similar to `Arc<Mutex<T>>` but with delegation semantics

## Architecture

### Core Components

1. **Trust<T>**: A thread-safe smart pointer that delegates all operations to a designated trustee fiber
2. **Trustee Fibers**: Each OS thread runs a trustee fiber that serializes access to entrusted properties
3. **SPSC Channels**: Request/response slots enable communication between client fibers and trustees
4. **Fiber Runtime**: Built on the `may` crate for lightweight user threads

## API Overview

### Basic Operations

```rust
use trust_tee::{LocalTrustee, Trust};

fn main() {
    let lt = LocalTrustee::new();
    let counter = lt.entrust(17i64);

    // Increment twice synchronously.
    counter.apply(|c| *c += 1);
    counter.apply(|c| *c += 1);

    // Read value.
    let v = counter.apply(|c| *c);
    assert_eq!(v, 19);

    // Non-blocking style (runs inline on local path).
    counter.apply_then(
        |c| {
            *c += 1;
            *c
        },
        |v| {
            assert_eq!(v, 20);
        },
    );

    let final_v = counter.apply(|c| *c);
    println!("final: {final_v}");
}
```

### Advanced Operations

```rust
use std::thread;

use trust_tee::RemoteRuntime;

// Non-capturing functions only (fn items) to remain allocation-free.
fn incr(c: &mut i64) {
    *c += 1;
}
fn get(c: &mut i64) -> u64 {
    *c as u64
}

fn main() {
    // Spawn a trustee thread managing a single counter with bounded queues.
    let (_rt, handle) = RemoteRuntime::spawn(0i64, 1024);

    // Single-threaded usage.
    handle.apply_mut(incr);
    let v = handle.apply_map_u64(get);
    assert_eq!(v, 1);

    // Multi-threaded clients (still SPSC per property; here we serialize per handle).
    let h2 = _rt.handle();
    let t1 = thread::spawn({
        let h = h2;
        move || {
            for _ in 0..10_000 {
                h.apply_mut(incr);
            }
        }
    });

    let h3 = _rt.handle();
    let t2 = thread::spawn({
        let h = h3;
        move || {
            for _ in 0..10_000 {
                h.apply_mut(incr);
            }
        }
    });

    t1.join().unwrap();
    t2.join().unwrap();

    let v = handle.apply_map_u64(get);
    println!("final: {v}");
    assert_eq!(v, 1 + 20_000);
}
```

## Examples

Can be viewed in [examples](./examples) directory

## References

This implementation is based on the paper:

> "Delegation with Trust<T>: A Scalable, Type- and Memory-Safe Alternative to Locks"

The design prioritizes the no-atomics approach described in the paper, using SPSC channels and trustee-side execution to achieve high performance without atomic instructions.

Arxiv: [Delegation with Trust<T>: A Scalable, Type- and Memory-Safe Alternative to Locks](https://arxiv.org/abs/2408.11173)

## License

This project is licensed under the MIT License. See the LICENSE file for details.
