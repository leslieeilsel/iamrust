#![no_main]

use iamrust_protocol::{ClientFrame, ServerFrame};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _client = serde_json::from_slice::<ClientFrame>(data);
    let _server = serde_json::from_slice::<ServerFrame>(data);
});
