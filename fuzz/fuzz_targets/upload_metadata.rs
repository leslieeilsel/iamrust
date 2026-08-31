#![no_main]

use iamrust_protocol::{CompleteUploadRequest, UploadAuthorizationRequest};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _authorization = serde_json::from_slice::<UploadAuthorizationRequest>(data);
    let _completion = serde_json::from_slice::<CompleteUploadRequest>(data);
});
