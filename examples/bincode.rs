use trust_tee::runtime::{
    serialization::{SlotWriter, decode_and_call, encode_closure},
    slots::*,
};

fn main() {
    // Client side
    let mut request_slot_buf = Aligned([0u8; SLOT_BYTES]);
    let mut req = SlotWriter::new(&mut request_slot_buf);
    let msg = String::from("hello");
    let header = encode_closure::<_, (), String>(
        &mut req,
        /*property_ptr*/ 0,
        move |_| msg + " world", // FnOnce consumes msg
        &[],
    );

    // Trustee side
    let result: String = unsafe { decode_and_call::<(), String>(&header, ()) };
    // result == "hello world"; msg was moved and dropped after call
    println!("Result: {}", result);
}
