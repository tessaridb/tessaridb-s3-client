use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use reqwest::header::{HeaderMap, HeaderValue};

use super::{Crc64Nvme, Verifier, crc64nvme};
use crate::Error;

const CHECK: &[u8] = b"123456789";

fn b64_u32(value: u32) -> String {
    STANDARD.encode(value.to_be_bytes())
}

fn headers(pairs: &[(&'static str, String)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        if let Ok(value) = HeaderValue::from_str(value) {
            map.insert(*name, value);
        }
    }
    map
}

#[test]
fn crc64nvme_matches_the_catalogued_check_value_whole_and_in_pieces() {
    // CRC-64/NVME of "123456789" is 0xAE8B14860A799888 (CRC catalogue).
    let expected = STANDARD.encode(0xAE8B_1486_0A79_9888_u64.to_be_bytes());
    assert_eq!(crc64nvme(CHECK), expected);
    let mut running = Crc64Nvme::new();
    running.update(&CHECK[..4]);
    running.update(&CHECK[4..]);
    assert_eq!(running.finish(), expected);
}

#[test]
fn every_declared_whole_object_checksum_is_checked() {
    let sha256 = STANDARD.encode(<sha2::Sha256 as sha2::Digest>::digest(CHECK));
    let sha1 = STANDARD.encode(<sha1::Sha1 as sha1::Digest>::digest(CHECK));
    let good = [
        ("x-amz-checksum-crc32", b64_u32(0xCBF4_3926)),
        ("x-amz-checksum-crc32c", b64_u32(0xE306_9283)),
        ("x-amz-checksum-crc64nvme", crc64nvme(CHECK)),
        ("x-amz-checksum-sha1", sha1),
        ("x-amz-checksum-sha256", sha256),
    ];
    let mut verifier = Verifier::from_headers(&headers(&good));
    verifier.update(&CHECK[..2]);
    verifier.update(&CHECK[2..]);
    assert!(verifier.finish().is_ok());
    for (index, (name, _)) in good.iter().enumerate() {
        let mut altered = good.clone();
        altered[index].1 = crc64nvme(b"something else");
        let mut verifier = Verifier::from_headers(&headers(&altered));
        verifier.update(CHECK);
        assert!(
            matches!(verifier.finish(), Err(Error::ChecksumMismatch { .. })),
            "{name}"
        );
    }
}

#[test]
fn a_composite_or_absent_checksum_is_not_checked() {
    let composite = [("x-amz-checksum-crc64nvme", "AAAAAAAAAAA=-3".to_owned())];
    let mut verifier = Verifier::from_headers(&headers(&composite));
    verifier.update(CHECK);
    assert!(verifier.finish().is_ok());
    let mut none = Verifier::from_headers(&HeaderMap::new());
    none.update(CHECK);
    assert!(none.finish().is_ok());
}
