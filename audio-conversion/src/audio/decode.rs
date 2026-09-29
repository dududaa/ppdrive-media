use crate::audio::decoded::DecodedAudio;
use crate::error::Error;
use crate::ffi::AvioReader;
use crate::ffi::Demuxer;

pub(crate) fn decode(input: &[u8]) -> Result<DecodedAudio, Error> {
    if input.is_empty() {
        return Err(Error::InvalidInput);
    }

    let reader = AvioReader::new(input)?;
    let mut demux = Demuxer::open(reader)?;

    let mut frames = Vec::new();
    let mut samples: i64 = 0;
    while let Some(frame) = demux.read_audio_frame()? {
        unsafe { (*frame.as_ptr()).pts = samples };
        samples += i64::from(frame.nb_samples());
        frames.push(frame);
    }

    if frames.is_empty() {
        return Err(Error::InvalidInput);
    }
    DecodedAudio::from_frames(frames)
}
