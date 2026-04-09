/// Decodes `data` as UTF-8, replacing invalid byte sequences with U+FFFD.
pub fn safe_decode(data: &[u8]) -> String {
    String::from_utf8_lossy(data).into_owned()
}
