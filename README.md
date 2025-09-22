# Trust<T>: A Delegation-Based Alternative to Locking

Trust<T> is a Rust library that provides a scalable, type- and memory-safe alternative to locking for concurrent programs. Instead of synchronizing access to shared objects with locks, Trust<T> delegates all operations to designated trustee fibers through a message-passing protocol.

## Overview

Trust<T> is based on **delegation**, a message-passing technique that avoids the scalability limitations of traditional locking. Key benefits include:

- **No atomic operations**: All coordination is done through single-producer/single-consumer (SPSC) channels
- **Higher throughput**: Per-object throughput is limited by trustee capacity rather than lock contention
- **Type safety**: Rust's type system ensures memory safety and prevents race conditions
- **Familiar API**: Similar to `Arc<Mutex<T>>` but with delegation semantics

## Architecture

### Core Components

1. **Trust<T>**: A thread-safe smart pointer that delegates all operations to a designated trustee fiber
2. **Trustee Fibers**: Each OS thread runs a trustee fiber that serializes access to entrusted properties
3. **SPSC Channels**: Request/response slots enable communication between client fibers and trustees
4. **Fiber Runtime**: Built on the `may` crate for lightweight user threads

### No-Atomics Design

Trust<T> operates entirely without atomic instructions:

- Single-writer/single-reader discipline for all channels
- Trustees poll for ready work using ordered loads/stores
- All shared state mutation occurs on the trustee thread
- Reference counting is managed by the trustee to avoid cross-thread atomics

## API Overview

### Basic Operations

```rust
use trustee::local_trustee;

// Entrust a property to a trustee
let trustee = local_trustee();
let counter = trustee.entrust(0i32);

// Apply operations synchronously
let result = counter.apply(|c| {
    *c += 1;
    *c
});

// Clone creates another reference to the same property
let counter_clone = counter.clone();
let value = counter_clone.apply(|c| *c); // Same shared state
```

### Advanced Operations

```rust
// Non-blocking delegation (not yet implemented)
counter.apply_then(
    |c| *c += 1,
    |result| println!("Result: {}", result)
);

// Variable-sized arguments (not yet implemented)
counter.apply_with(
    |c, data| *c += data.len(),
    vec![1, 2, 3]
);

// Trustee-side fibers for blocking operations (not yet implemented)
let latched = trustee.entrust(Latch::new(data));
latched.launch(|d| {
    // Can perform blocking operations
    d.apply(some_blocking_operation);
});
```

## Examples

Run the fetch-and-add example to see Trust<T> in action:

```bash
cargo run --example fetch_and_add
```

This demonstrates:

- Basic property entrustment and delegation
- State persistence across multiple operations
- Reference counting with cloned Trust instances
- Complex operations on entrusted data structures

## Architecture Details

### Trust<T> Structure

```rust
pub struct Trust<T> {
    trustee_ref: TrusteeReference,  // Reference to owning trustee
    property_id: PropertyId,        // Unique property identifier
    _phantom: PhantomData<T>,       // Type information
}
```

### Trustee State Management

Each thread maintains a trustee state containing:

- **Properties**: HashMap of property_id -> Box<dyn Any + Send>
- **Reference Counts**: HashMap of property_id -> usize

### Request/Response Slots

Communication uses dedicated SPSC slots with:

- **Primary Block**: 128 bytes for headers and small requests
- **Overflow Block**: 1024 bytes for larger batches
- **Ready Bits**: Single-bit flags to indicate new data
- **Batching**: Multiple requests per slot for efficiency

## Performance Characteristics

Based on the research paper this implementation follows:

- **Congested Workloads**: 8-22× better throughput than best locks
- **Uncongested Workloads**: Competitive with locks (slightly higher latency)
- **Scalability**: Per-object throughput limited by trustee capacity, not lock contention
- **Memory**: Excellent cache locality due to trustee-local execution

## Safety Guarantees

Trust<T> provides several safety guarantees through Rust's type system:

1. **Memory Safety**: No data races or use-after-free bugs
2. **Type Safety**: Closures cannot capture references or raw pointers
3. **Send/Sync**: Proper trait bounds ensure thread safety
4. **'static Lifetimes**: All delegated closures must be 'static

## Limitations

Current implementation limitations:

1. **Local Trustee Only**: Remote delegation not yet implemented
2. **Basic API**: Only apply() is fully functional
3. **No Fibers**: Full fiber runtime integration pending
4. **No Benchmarks**: Performance validation tests not implemented

## Future Work

The roadmap includes:

1. **Complete Remote Delegation**: Full cross-thread delegation protocol
2. **Fiber Integration**: Complete may runtime integration with suspension/resumption
3. **Advanced API**: apply_then(), apply_with(), and launch() implementations
4. **Performance Testing**: Microbenchmarks and application-level evaluations
5. **Optimizations**: Advanced scheduling heuristics and load balancing

## References

This implementation is based on the paper:

> "Delegation with Trust<T>: A Scalable, Type- and Memory-Safe Alternative to Locks"

The design prioritizes the no-atomics approach described in the paper, using SPSC channels and trustee-side execution to achieve high performance without atomic instructions.

Arxiv: [Delegation with Trust<T>: A Scalable, Type- and Memory-Safe Alternative to Locks](https://arxiv.org/abs/2408.11173)

## License

This project is licensed under the MIT License. See the LICENSE file for details.
