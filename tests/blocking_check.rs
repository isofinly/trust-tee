use trust_tee::prelude::*;


#[test]
fn test_is_in_delegated_context_remote() {
    let remote = Remote::entrust(0);
    // launch runs on the trustee thread. We verify that the context flag is set.
    let is_delegated = remote.launch(|_| {
        trust_tee::util::fiber::is_in_delegated_context()
    });
    assert!(is_delegated, "Should be in delegated context during launch");
}

#[test]
fn test_not_in_delegated_context_main() {
    assert!(!trust_tee::util::fiber::is_in_delegated_context(), "Should not be in delegated context on main thread");
}

#[test]
fn test_blocking_in_launch_panics_caught() {
    let remote = Remote::entrust(0);
    let remote_clone = remote.clone();

    // launch runs on the trustee thread.
    remote.launch(move |_| {
        // We expect this to panic.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            remote_clone.apply(|_| {});
        }));
        assert!(result.is_err(), "Should have panicked due to blocking in delegated context");
    });
}
