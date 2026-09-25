use crate::error::Error;
use crate::ffi::wrappers::Frame;
use crate::ffi::{AvioReader, Demuxer};

fn is_avif(data: &[u8]) -> bool {
    if data.len() < 12 || &data[4..8] != b"ftyp" {
        return false;
    }
    matches!(&data[8..12], b"avif" | b"avis" | b"mif1" | b"miaf")
}

pub(crate) fn decode(input: &[u8]) -> Result<Frame, Error> {
    if input.is_empty() {
        return Err(Error::InvalidInput);
    }
    if is_avif(input) {
        return Err(Error::UnsupportedFormat);
    }

    let reader = AvioReader::new(input)?;
    let mut demux = Demuxer::open(reader)?;
    demux.read_video_frame()
}
