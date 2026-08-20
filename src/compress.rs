use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use std::io::Write;

pub(crate) fn compress(s: &str) -> anyhow::Result<Vec<u8>> {
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(s.as_bytes())?;
    Ok(gz.finish()?)
}

pub(crate) fn decompress_json<T>(buf: &Vec<u8>) -> anyhow::Result<T>
where
    for<'a> T: serde::Deserialize<'a>,
{
    let reader = GzDecoder::new(buf.as_slice());
    Ok(serde_json::from_reader(reader)?)
}
