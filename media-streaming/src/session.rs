use std::path::Path;
use std::ptr;

use audio_conversion::{EncodedPacket, EncoderParams};
use video_conversion::{VideoEncoderParams, VideoPacket};

use crate::error::Error;
use crate::ffi;
use crate::ffi::{AVDictionary, AVFormatContext, AVPacket, AVRational};
use crate::options::StreamingProtocol;

/// One open `hls`/`dash` output context: streams are declared first,
/// then the header, then packets, then the trailer finalizes the
/// playlists. Segments are written by FFmpeg itself to the playlist's
/// directory (file-path output).
pub(crate) struct SegmentSession {
    fmt: *mut AVFormatContext,
    packet: *mut AVPacket,
    header_written: bool,
    trailer_written: bool,
}

impl SegmentSession {
    pub fn new(protocol: StreamingProtocol, playlist: &Path) -> Result<SegmentSession, Error> {
        let muxer = ffi::to_cstr(match protocol {
            StreamingProtocol::Hls => "hls",
            StreamingProtocol::Dash => "dash",
        })?;
        // HLS variant playlists are named `stream_%v.m3u8` next to the
        // master playlist (the muxer requires `%v` once a variant map
        // is set); DASH writes everything relative to the manifest.
        let variant_base = match protocol {
            StreamingProtocol::Hls => playlist
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join("stream_%v.m3u8"),
            StreamingProtocol::Dash => playlist.to_path_buf(),
        };
        let url = path_to_cstr(&variant_base)?;
        unsafe {
            let mut fmt: *mut AVFormatContext = ptr::null_mut();
            let ret = ffi::avformat_alloc_output_context2(
                &mut fmt,
                ptr::null_mut(),
                muxer.as_ptr(),
                url.as_ptr(),
            );
            if ret < 0 || fmt.is_null() {
                return Err(Error::from_code(ret));
            }
            let packet = ffi::av_packet_alloc();
            if packet.is_null() {
                ffi::avformat_free_context(fmt);
                return Err(Error::FfmpegError(
                    "av_packet_alloc returned null".to_string(),
                ));
            }
            Ok(SegmentSession {
                fmt,
                packet,
                header_written: false,
                trailer_written: false,
            })
        }
    }

    /// Declares one video rendition stream from an open packetized
    /// encoder's params; `bitrate` feeds the playlist's bandwidth
    /// attributes and `sar` is the pixel aspect ratio that keeps the
    /// rendition's display aspect ratio equal to the source's (the
    /// DASH muxer rejects an adaptation set whose members disagree).
    pub fn add_video_stream(
        &mut self,
        params: &VideoEncoderParams,
        bitrate: u64,
        sar: (i32, i32),
    ) -> Result<usize, Error> {
        let codec_id = ffi::codec_id_by_name(&params.codec_name)?;
        unsafe {
            let stream = self.new_stream()?;
            let par = (*stream).codecpar;
            (*par).codec_type = ffi::AVMEDIA_TYPE_VIDEO;
            (*par).codec_id = codec_id;
            (*par).width = params.width as i32;
            (*par).height = params.height as i32;
            let pix_fmt = ffi::to_cstr(&params.pix_fmt)?;
            (*par).format = ffi::av_get_pix_fmt(pix_fmt.as_ptr()) as i32;
            (*par).bit_rate = bitrate as i64;
            ffi::set_extradata(par, &params.extradata)?;
            let sar = AVRational {
                num: sar.0,
                den: sar.1,
            };
            (*par).sample_aspect_ratio = sar;
            (*stream).sample_aspect_ratio = sar;
            (*stream).time_base = AVRational {
                num: params.time_base_num,
                den: params.time_base_den,
            };
            Ok((*stream).index as usize)
        }
    }

    /// Declares the shared audio rendition stream.
    pub fn add_audio_stream(
        &mut self,
        params: &EncoderParams,
        bitrate: u64,
    ) -> Result<usize, Error> {
        let codec_id = ffi::codec_id_by_name(&params.codec_name)?;
        unsafe {
            let stream = self.new_stream()?;
            let par = (*stream).codecpar;
            (*par).codec_type = ffi::AVMEDIA_TYPE_AUDIO;
            (*par).codec_id = codec_id;
            (*par).sample_rate = params.sample_rate as i32;
            let sample_fmt = ffi::to_cstr(&params.sample_fmt)?;
            (*par).format = ffi::av_get_sample_fmt(sample_fmt.as_ptr()) as i32;
            ffi::av_channel_layout_default(&mut (*par).ch_layout, i32::from(params.channels));
            (*par).bit_rate = bitrate as i64;
            ffi::set_extradata(par, &params.extradata)?;
            (*stream).time_base = AVRational {
                num: params.time_base_num,
                den: params.time_base_den,
            };
            Ok((*stream).index as usize)
        }
    }

    /// Writes the muxer header with the given options (protocol
    /// layout: `var_stream_map`/`adaptation_sets`, segment duration,
    /// segment naming). Must run after every stream is added and
    /// before the first packet.
    pub fn write_header(&mut self, options: &[(String, String)]) -> Result<(), Error> {
        let mut dict: *mut AVDictionary = ptr::null_mut();
        unsafe {
            for (key, value) in options {
                let key = ffi::to_cstr(key)?;
                let value = ffi::to_cstr(value)?;
                let ret = ffi::av_dict_set(&mut dict, key.as_ptr(), value.as_ptr(), 0);
                if ret < 0 {
                    ffi::av_dict_free(&mut dict);
                    return Err(Error::from_code(ret));
                }
            }
            let ret = ffi::avformat_write_header(self.fmt, &mut dict);
            ffi::av_dict_free(&mut dict);
            if ret < 0 {
                return Err(Error::from_code(ret));
            }
        }
        self.header_written = true;
        Ok(())
    }

    pub fn write_video(&mut self, index: usize, packet: &VideoPacket) -> Result<(), Error> {
        self.write_raw(
            index,
            &packet.data,
            packet.pts,
            packet.dts,
            packet.duration,
            packet.tb_num,
            packet.tb_den,
            packet.is_keyframe,
        )
    }

    pub fn write_audio(&mut self, index: usize, packet: &EncodedPacket) -> Result<(), Error> {
        self.write_raw(
            index,
            &packet.data,
            packet.pts,
            packet.pts,
            packet.duration,
            packet.tb_num,
            packet.tb_den,
            true,
        )
    }

    /// Writes the trailer, finalizing the playlists (e.g. the HLS
    /// `#EXT-X-ENDLIST`), and consumes the session.
    pub fn finish(mut self) -> Result<(), Error> {
        unsafe {
            let ret = ffi::av_write_trailer(self.fmt);
            self.trailer_written = true;
            if ret < 0 {
                return Err(Error::from_code(ret));
            }
        }
        Ok(())
    }

    unsafe fn new_stream(&mut self) -> Result<*mut ffi::AVStream, Error> {
        unsafe {
            let stream = ffi::avformat_new_stream(self.fmt, ptr::null());
            if stream.is_null() {
                return Err(Error::FfmpegError(
                    "avformat_new_stream returned null".to_string(),
                ));
            }
            Ok(stream)
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn write_raw(
        &mut self,
        index: usize,
        data: &[u8],
        pts: i64,
        dts: i64,
        duration: i64,
        tb_num: i32,
        tb_den: i32,
        keyframe: bool,
    ) -> Result<(), Error> {
        unsafe {
            ffi::av_packet_unref(self.packet);
            let ret = ffi::av_new_packet(self.packet, data.len() as i32);
            if ret < 0 {
                return Err(Error::from_code(ret));
            }
            ptr::copy_nonoverlapping(data.as_ptr(), (*self.packet).data, data.len());
            let stream = *(*self.fmt).streams.add(index);
            let source = AVRational {
                num: tb_num,
                den: tb_den,
            };
            let target = (*stream).time_base;
            (*self.packet).pts = ffi::av_rescale_q(pts, source, target);
            (*self.packet).dts = ffi::av_rescale_q(dts, source, target);
            (*self.packet).duration = ffi::av_rescale_q(duration, source, target);
            (*self.packet).time_base = target;
            (*self.packet).stream_index = index as i32;
            (*self.packet).pos = -1;
            if keyframe {
                (*self.packet).flags |= ffi::AV_PKT_FLAG_KEY as i32;
            } else {
                (*self.packet).flags &= !(ffi::AV_PKT_FLAG_KEY as i32);
            }
            let ret = ffi::av_write_frame(self.fmt, self.packet);
            if ret < 0 {
                return Err(Error::from_code(ret));
            }
            Ok(())
        }
    }
}

impl Drop for SegmentSession {
    fn drop(&mut self) {
        unsafe {
            if self.header_written && !self.trailer_written {
                let _ = ffi::av_write_trailer(self.fmt);
            }
            if !self.packet.is_null() {
                ffi::av_packet_free(&mut self.packet);
            }
            if !self.fmt.is_null() {
                ffi::avformat_free_context(self.fmt);
            }
        }
    }
}

/// HLS header options: master playlist name, VOD playlist, fixed
/// segment duration, TS segment naming (`%v` expands per variant),
/// independent-segment signalling and the rendition map (variants are
/// space-separated, e.g. `v:0,a:0 v:1,a:1` — the muxer rejects a
/// shared audio stream in two variants, so audio is duplicated per
/// variant).
pub(crate) fn hls_header_options(
    segment_duration: u32,
    var_stream_map: &str,
    playlist: &Path,
) -> Vec<(String, String)> {
    let dir = playlist.parent().unwrap_or_else(|| Path::new("."));
    let pattern = dir.join("seg_%v_%03d.ts");
    vec![
        ("master_pl_name".to_string(), "master.m3u8".to_string()),
        ("hls_time".to_string(), segment_duration.to_string()),
        ("hls_playlist_type".to_string(), "vod".to_string()),
        ("hls_list_size".to_string(), "0".to_string()),
        (
            "hls_segment_filename".to_string(),
            pattern.to_string_lossy().into_owned(),
        ),
        ("hls_flags".to_string(), "independent_segments".to_string()),
        ("var_stream_map".to_string(), var_stream_map.to_string()),
    ]
}

/// DASH header options: VOD manifest with template naming and one
/// adaptation set for video plus one for audio (or whichever exists).
pub(crate) fn dash_header_options(
    segment_duration: u32,
    has_video: bool,
    has_audio: bool,
) -> Vec<(String, String)> {
    let sets = match (has_video, has_audio) {
        (true, true) => "id=0,streams=v id=1,streams=a",
        (true, false) => "id=0,streams=v",
        (false, true) => "id=0,streams=a",
        (false, false) => "",
    };
    vec![
        ("seg_duration".to_string(), segment_duration.to_string()),
        ("use_template".to_string(), "1".to_string()),
        ("window_size".to_string(), "0".to_string()),
        ("remove_at_exit".to_string(), "0".to_string()),
        ("adaptation_sets".to_string(), sets.to_string()),
    ]
}

fn path_to_cstr(path: &Path) -> Result<std::ffi::CString, Error> {
    path.to_str()
        .ok_or(Error::InvalidInput)
        .and_then(ffi::to_cstr)
}
