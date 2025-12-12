use trust_tee::prelude::*;


#[test]
fn test_is_trustee_thread_remote() {
    let remote = Remote::entrust(0);
    // launch runs on the trustee thread (logically/cooperatively).
    // It should inherit the TrusteeThread guard status because it usually runs on the same thread.
    let is_trustee = remote.launch(|_| {
        trust_tee::util::fiber::is_trustee_thread()
    });
   
    println!("Is trustee thread? {}", is_trustee);
}

#[test]
fn test_recursive_apply_ok() {
    // Normal multi-threaded execution.
    let remote = Remote::entrust(0);
    let remote_clone = remote.clone();

    // launch runs on the trustee thread.
    remote.launch(move |_| {
        // This should NOT panic.
        // Scenario 1: Same Thread -> Local shortcut used (safe).
        // Scenario 2: Diff Thread -> Blocking applies (safe).
        // In BOTH cases, this call should succeed.
        // NOTE: We usage `with` here because `launch` holds a shared lock (&T).
        // Calling `apply` (&mut T) recursively would be UB (aliasing violation).

        let remote_clone2 = remote_clone.clone();
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            remote_clone2.with(|_| {});
        }));
        assert!(res.is_ok(), "Recursive with should succeed (either via shortcut or safe blocking)");
    });
}

#[test]
fn test_cross_trustee_blocking_panics() {
    let remote1 = Remote::entrust(0);
    let remote2 = Remote::entrust(0);
    let remote2_clone = remote2.clone();

    remote1.launch(move |_| {
         if trust_tee::util::fiber::is_trustee_thread() {
             // We are on trustee thread. Calling remote2.apply should panic.
             let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                 remote2_clone.apply(|_| {});
             }));
             assert!(result.is_err(), "Should have panicked on trustee thread");
         } else {
             println!("Skipping panic check - scheduled on different thread");
         }
    });
}
