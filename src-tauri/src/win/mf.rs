//! Media Foundation helpers: a frame reader (decode any video Windows can play to
//! packed BGRA, optionally scaled), a PCM audio reader, and an MP4 writer (H.264 /
//! HEVC + AAC, hardware encoders when available). These replace AVAssetReader /
//! AVAssetWriter from the macOS build.

use std::sync::OnceLock;
use windows::core::{GUID, HSTRING};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;

/// 100-ns ticks per second (Media Foundation's time unit).
const HNS: f64 = 10_000_000.0;

/// CODECAPI_AVEncMPVGOPSize
const GOP_SIZE: GUID = GUID::from_u128(0x95f31b26_95a4_41aa_9303_246a7fc6eef1);

pub fn startup() {
    static STARTED: OnceLock<bool> = OnceLock::new();
    STARTED.get_or_init(|| unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL).is_ok() });
}

fn err(e: windows::core::Error) -> String {
    e.message().to_string()
}

fn attributes(n: u32) -> Result<IMFAttributes, String> {
    let mut a: Option<IMFAttributes> = None;
    unsafe { MFCreateAttributes(&mut a, n).map_err(err)? };
    a.ok_or_else(|| "MFCreateAttributes failed".into())
}

fn pack(hi: u32, lo: u32) -> u64 {
    ((hi as u64) << 32) | lo as u64
}

fn unpack(v: u64) -> (u32, u32) {
    ((v >> 32) as u32, v as u32)
}

/// Frame rate as a rational close to `fps` (29.97 -> 30000/1001 style precision).
fn rate(fps: f64) -> (u32, u32) {
    let fps = if fps.is_finite() && fps > 0.0 { fps } else { 30.0 };
    ((fps * 1000.0).round() as u32, 1000)
}

const VIDEO: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
const AUDIO: u32 = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;

fn open_reader(path: &str, video_processing: bool) -> Result<IMFSourceReader, String> {
    startup();
    let attrs = attributes(2)?;
    unsafe {
        if video_processing {
            attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1).map_err(err)?;
        }
        let r = MFCreateSourceReaderFromURL(&HSTRING::from(path), &attrs).map_err(err)?;
        r.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false).map_err(err)?;
        Ok(r)
    }
}

fn duration_of(reader: &IMFSourceReader) -> f64 {
    unsafe {
        reader
            .GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)
            .ok()
            .and_then(|v| u64::try_from(&v).ok())
            .map(|d| d as f64 / HNS)
            .unwrap_or(0.0)
    }
}

fn seek(reader: &IMFSourceReader, t_s: f64) -> Result<(), String> {
    let pos = PROPVARIANT::from((t_s.max(0.0) * HNS) as i64);
    unsafe { reader.SetCurrentPosition(&GUID::zeroed(), &pos).map_err(err) }
}

// ------------------------------------------------------------------ probing

#[derive(Clone, Debug, Default)]
pub struct Info {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration_s: f64,
    /// Video bits per second, 0 when unknown.
    pub video_bps: f64,
    pub has_audio: bool,
}

pub fn probe(path: &str) -> Result<Info, String> {
    let reader = open_reader(path, false)?;
    unsafe {
        let native = reader.GetNativeMediaType(VIDEO, 0).map_err(|_| "no video track found".to_string())?;
        let (w, h) = unpack(native.GetUINT64(&MF_MT_FRAME_SIZE).map_err(err)?);
        let fps = native
            .GetUINT64(&MF_MT_FRAME_RATE)
            .ok()
            .map(unpack)
            .filter(|(_, d)| *d > 0)
            .map(|(n, d)| n as f64 / d as f64)
            .unwrap_or(0.0);
        let video_bps = native.GetUINT32(&MF_MT_AVG_BITRATE).map(|b| b as f64).unwrap_or(0.0);
        let has_audio = reader.GetNativeMediaType(AUDIO, 0).is_ok();
        Ok(Info { width: w, height: h, fps, duration_s: duration_of(&reader), video_bps, has_audio })
    }
}

// ------------------------------------------------------------------ video reader

pub struct Frame {
    /// Presentation time in seconds.
    pub pts: f64,
    /// Tightly packed BGRA (alpha undefined).
    pub data: Vec<u8>,
    pub w: u32,
    pub h: u32,
}

pub struct VideoReader {
    reader: IMFSourceReader,
    pub width: u32,
    pub height: u32,
    pub native_w: u32,
    pub native_h: u32,
    pub duration: f64,
    stride: i32,
    eof: bool,
}

impl VideoReader {
    /// Open the first video track. `width`: scale to this width (keeping the aspect
    /// ratio, rounded to even); `None` decodes at the native size.
    pub fn open(path: &str, width: Option<u32>) -> Result<Self, String> {
        let reader = open_reader(path, true)?;
        unsafe {
            reader.SetStreamSelection(VIDEO, true).map_err(err)?;
            let native = reader.GetNativeMediaType(VIDEO, 0).map_err(|_| "no video track".to_string())?;
            let (nw, nh) = unpack(native.GetUINT64(&MF_MT_FRAME_SIZE).map_err(err)?);
            if nw < 2 || nh < 2 {
                return Err("empty video".into());
            }
            let want = |w: u32, h: u32| -> Result<IMFMediaType, String> {
                let t = MFCreateMediaType().map_err(err)?;
                t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).map_err(err)?;
                t.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32).map_err(err)?;
                if w > 0 {
                    t.SetUINT64(&MF_MT_FRAME_SIZE, pack(w, h)).map_err(err)?;
                }
                Ok(t)
            };
            let scaled = width.filter(|&w| w < nw).map(|w| {
                let w = w & !1;
                (w, ((w as f64 * nh as f64 / nw as f64).round() as u32) & !1)
            });
            // ask the video processor to scale; fall back to native size (callers rescale)
            let mut ok = false;
            if let Some((w, h)) = scaled {
                ok = reader.SetCurrentMediaType(VIDEO, None, &want(w, h)?).is_ok();
            }
            if !ok {
                reader
                    .SetCurrentMediaType(VIDEO, None, &want(0, 0)?)
                    .map_err(|e| format!("cannot decode this video: {}", err(e)))?;
            }
            let mut r = VideoReader {
                reader,
                width: 0,
                height: 0,
                native_w: nw,
                native_h: nh,
                duration: 0.0,
                stride: 0,
                eof: false,
            };
            r.duration = duration_of(&r.reader);
            r.refresh_type()?;
            Ok(r)
        }
    }

    fn refresh_type(&mut self) -> Result<(), String> {
        unsafe {
            let t = self.reader.GetCurrentMediaType(VIDEO).map_err(err)?;
            let (w, h) = unpack(t.GetUINT64(&MF_MT_FRAME_SIZE).map_err(err)?);
            self.width = w;
            self.height = h;
            self.stride = t.GetUINT32(&MF_MT_DEFAULT_STRIDE).map(|s| s as i32).unwrap_or(w as i32 * 4);
        }
        Ok(())
    }

    pub fn seek(&mut self, t_s: f64) -> Result<(), String> {
        self.eof = false;
        seek(&self.reader, t_s)
    }

    /// Next decoded frame, or `None` at the end of the stream (or on a read error).
    pub fn next(&mut self) -> Option<Frame> {
        if self.eof {
            return None;
        }
        unsafe {
            loop {
                let mut flags = 0u32;
                let mut ts = 0i64;
                let mut sample: Option<IMFSample> = None;
                if self
                    .reader
                    .ReadSample(VIDEO, 0, None, Some(&mut flags), Some(&mut ts), Some(&mut sample))
                    .is_err()
                {
                    self.eof = true;
                    return None;
                }
                if flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32 != 0 {
                    let _ = self.refresh_type();
                }
                if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                    self.eof = true;
                    return None;
                }
                let Some(sample) = sample else { continue };
                if let Some(data) = self.copy_out(&sample) {
                    return Some(Frame { pts: ts as f64 / HNS, data, w: self.width, h: self.height });
                }
            }
        }
    }

    unsafe fn copy_out(&self, sample: &IMFSample) -> Option<Vec<u8>> {
        let (w, h) = (self.width as usize, self.height as usize);
        let row = w * 4;
        let mut out = vec![0u8; row * h];
        let buf = sample.GetBufferByIndex(0).ok()?;
        // 2D buffers know their real pitch (and whether the image is bottom-up)
        if let Ok(b2) = windows::core::Interface::cast::<IMF2DBuffer>(&buf) {
            let mut p: *mut u8 = std::ptr::null_mut();
            let mut pitch = 0i32;
            if b2.Lock2D(&mut p, &mut pitch).is_ok() {
                for y in 0..h {
                    let src = p.offset(y as isize * pitch as isize);
                    std::ptr::copy_nonoverlapping(src, out.as_mut_ptr().add(y * row), row);
                }
                let _ = b2.Unlock2D();
                return Some(out);
            }
        }
        let buf = sample.ConvertToContiguousBuffer().ok()?;
        let mut p: *mut u8 = std::ptr::null_mut();
        let mut len = 0u32;
        buf.Lock(&mut p, None, Some(&mut len)).ok()?;
        let pitch = if self.stride == 0 { row as i32 } else { self.stride };
        let base = if pitch < 0 { p.add((h - 1) * row) } else { p };
        if (len as usize) >= row * h {
            for y in 0..h {
                let src = base.offset(y as isize * pitch as isize);
                std::ptr::copy_nonoverlapping(src, out.as_mut_ptr().add(y * row), row);
            }
        }
        let _ = buf.Unlock();
        Some(out)
    }
}

// ------------------------------------------------------------------ audio reader

/// Decodes the first audio track to interleaved 16-bit stereo PCM at 48 kHz.
pub struct AudioReader {
    reader: IMFSourceReader,
    pub rate: u32,
    pub channels: u32,
}

impl AudioReader {
    pub fn open(path: &str) -> Result<Self, String> {
        let reader = open_reader(path, false)?;
        unsafe {
            reader.SetStreamSelection(AUDIO, true).map_err(err)?;
            let t = MFCreateMediaType().map_err(err)?;
            t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio).map_err(err)?;
            t.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM).map_err(err)?;
            t.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16).map_err(err)?;
            t.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, 48_000).map_err(err)?;
            t.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2).map_err(err)?;
            if reader.SetCurrentMediaType(AUDIO, None, &t).is_err() {
                // no resampler for this source: take 16-bit PCM in its own layout
                let t = MFCreateMediaType().map_err(err)?;
                t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio).map_err(err)?;
                t.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM).map_err(err)?;
                t.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16).map_err(err)?;
                reader.SetCurrentMediaType(AUDIO, None, &t).map_err(err)?;
            }
            let cur = reader.GetCurrentMediaType(AUDIO).map_err(err)?;
            let rate = cur.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND).unwrap_or(48_000);
            let channels = cur.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS).unwrap_or(2);
            Ok(Self { reader, rate, channels })
        }
    }

    /// Next chunk as (pts seconds, interleaved samples in the reader's channel layout).
    pub fn next(&mut self) -> Option<(f64, Vec<i16>)> {
        unsafe {
            loop {
                let mut flags = 0u32;
                let mut ts = 0i64;
                let mut sample: Option<IMFSample> = None;
                self.reader
                    .ReadSample(AUDIO, 0, None, Some(&mut flags), Some(&mut ts), Some(&mut sample))
                    .ok()?;
                if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                    return None;
                }
                let Some(sample) = sample else { continue };
                let buf = sample.ConvertToContiguousBuffer().ok()?;
                let mut p: *mut u8 = std::ptr::null_mut();
                let mut len = 0u32;
                buf.Lock(&mut p, None, Some(&mut len)).ok()?;
                let pcm = std::slice::from_raw_parts(p as *const i16, len as usize / 2).to_vec();
                let _ = buf.Unlock();
                return Some((ts as f64 / HNS, pcm));
            }
        }
    }
}

// ------------------------------------------------------------------ writer

pub struct VideoSpec {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub bitrate: u32,
    pub hevc: bool,
    /// Keyframe interval in frames (0 = encoder default).
    pub gop: u32,
}

pub struct AudioSpec {
    pub rate: u32,
    pub channels: u32,
    /// AAC bit rate; the encoder accepts 96, 128, 160 or 192 kbps.
    pub kbps: u32,
}

/// MP4 writer fed with BGRA frames and 16-bit PCM.
pub struct Writer {
    writer: IMFSinkWriter,
    path: String,
    fast_start: bool,
    video: u32,
    audio: Option<u32>,
    w: u32,
    h: u32,
    frame_hns: i64,
    audio_rate: u32,
    audio_channels: u32,
}

fn aac_kbps(kbps: u32) -> u32 {
    // nearest supported bit rate
    *[96u32, 128, 160, 192].iter().min_by_key(|v| (**v as i64 - kbps as i64).abs()).unwrap()
}

impl Writer {
    pub fn create(path: &str, v: &VideoSpec, a: Option<&AudioSpec>, fast_start: bool) -> Result<Self, String> {
        startup();
        let _ = std::fs::remove_file(path);
        unsafe {
            let attrs = attributes(3)?;
            attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1).map_err(err)?;
            // otherwise WriteSample blocks a stream that runs ahead of the other one
            attrs.SetUINT32(&MF_SINK_WRITER_DISABLE_THROTTLING, 1).map_err(err)?;
            attrs.SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4).map_err(err)?;
            // (MF_MPEG4SINK_MOOV_BEFORE_MDAT writes broken files; see `faststart` instead)
            let writer = MFCreateSinkWriterFromURL(&HSTRING::from(path), None, &attrs)
                .map_err(|e| format!("cannot create {path}: {}", err(e)))?;

            let (fr_n, fr_d) = rate(v.fps);
            let common = |t: &IMFMediaType| -> Result<(), String> {
                t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).map_err(err)?;
                t.SetUINT64(&MF_MT_FRAME_SIZE, pack(v.width, v.height)).map_err(err)?;
                t.SetUINT64(&MF_MT_FRAME_RATE, pack(fr_n, fr_d)).map_err(err)?;
                t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1)).map_err(err)?;
                t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32).map_err(err)?;
                // matches `bgra_to_nv12`, and is flagged in the stream for decoders
                t.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709.0 as u32).map_err(err)?;
                t.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32).map_err(err)?;
                t.SetUINT32(&MF_MT_VIDEO_PRIMARIES, MFVideoPrimaries_BT709.0 as u32).map_err(err)?;
                t.SetUINT32(&MF_MT_TRANSFER_FUNCTION, MFVideoTransFunc_709.0 as u32).map_err(err)?;
                Ok(())
            };
            let out = MFCreateMediaType().map_err(err)?;
            common(&out)?;
            out.SetGUID(&MF_MT_SUBTYPE, if v.hevc { &MFVideoFormat_HEVC } else { &MFVideoFormat_H264 }).map_err(err)?;
            out.SetUINT32(&MF_MT_AVG_BITRATE, v.bitrate.max(100_000)).map_err(err)?;
            if !v.hevc {
                out.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_High.0 as u32).map_err(err)?;
            }
            let video = writer.AddStream(&out).map_err(|e| {
                format!(
                    "the {} encoder does not support {}x{}: {}",
                    if v.hevc { "HEVC" } else { "H.264" },
                    v.width,
                    v.height,
                    err(e)
                )
            })?;
            let inp = MFCreateMediaType().map_err(err)?;
            common(&inp)?;
            // NV12 goes straight into the encoder; with RGB32 the sink writer inserts a
            // software colour converter that cannot keep up with 1080p60
            inp.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12).map_err(err)?;
            inp.SetUINT32(&MF_MT_DEFAULT_STRIDE, v.width).map_err(err)?;
            let params = attributes(1)?;
            if v.gop > 0 {
                params.SetUINT32(&GOP_SIZE, v.gop).map_err(err)?;
            }
            writer
                .SetInputMediaType(video, &inp, &params)
                .map_err(|e| format!("this codec/resolution combination is not supported by the encoder: {}", err(e)))?;

            let mut audio = None;
            let (mut audio_rate, mut audio_channels) = (48_000, 2);
            if let Some(a) = a {
                audio_rate = if a.rate == 44_100 { 44_100 } else { 48_000 };
                audio_channels = a.channels.clamp(1, 2);
                let out = MFCreateMediaType().map_err(err)?;
                out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio).map_err(err)?;
                out.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC).map_err(err)?;
                out.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16).map_err(err)?;
                out.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, audio_rate).map_err(err)?;
                out.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, audio_channels).map_err(err)?;
                out.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, aac_kbps(a.kbps) * 1000 / 8).map_err(err)?;
                let idx = writer.AddStream(&out).map_err(|e| format!("AAC encoder unavailable: {}", err(e)))?;
                let inp = MFCreateMediaType().map_err(err)?;
                inp.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio).map_err(err)?;
                inp.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM).map_err(err)?;
                inp.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16).map_err(err)?;
                inp.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, audio_rate).map_err(err)?;
                inp.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, audio_channels).map_err(err)?;
                inp.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, audio_channels * 2).map_err(err)?;
                inp.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, audio_rate * audio_channels * 2).map_err(err)?;
                writer.SetInputMediaType(idx, &inp, None).map_err(err)?;
                audio = Some(idx);
            }
            writer.BeginWriting().map_err(err)?;
            Ok(Self {
                writer,
                path: path.to_string(),
                fast_start,
                video,
                audio,
                w: v.width,
                h: v.height,
                frame_hns: (HNS / v.fps.max(1.0)) as i64,
                audio_rate,
                audio_channels,
            })
        }
    }

    pub fn audio_format(&self) -> Option<(u32, u32)> {
        self.audio.map(|_| (self.audio_rate, self.audio_channels))
    }

    unsafe fn sample(bytes: &[u8], t_s: f64, dur_hns: i64) -> Result<IMFSample, String> {
        let buf = MFCreateMemoryBuffer(bytes.len() as u32).map_err(err)?;
        let mut p: *mut u8 = std::ptr::null_mut();
        buf.Lock(&mut p, None, None).map_err(err)?;
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        buf.Unlock().map_err(err)?;
        buf.SetCurrentLength(bytes.len() as u32).map_err(err)?;
        let s = MFCreateSample().map_err(err)?;
        s.AddBuffer(&buf).map_err(err)?;
        s.SetSampleTime((t_s.max(0.0) * HNS) as i64).map_err(err)?;
        s.SetSampleDuration(dur_hns.max(1)).map_err(err)?;
        Ok(s)
    }

    /// `bgra`: tightly packed, exactly the writer's width x height.
    pub fn write_video(&mut self, bgra: &[u8], t_s: f64) -> Result<(), String> {
        if bgra.len() != (self.w * self.h * 4) as usize {
            return Err("frame size mismatch".into());
        }
        let nv12 = bgra_to_nv12(bgra, self.w as usize, self.h as usize);
        unsafe {
            let s = Self::sample(&nv12, t_s, self.frame_hns)?;
            self.writer.WriteSample(self.video, &s).map_err(err)
        }
    }

    /// Interleaved PCM in the writer's audio format, starting at `t_s`.
    pub fn write_audio(&mut self, pcm: &[i16], t_s: f64) -> Result<(), String> {
        let Some(idx) = self.audio else { return Ok(()) };
        if pcm.is_empty() {
            return Ok(());
        }
        let frames = pcm.len() as f64 / self.audio_channels as f64;
        let dur = (frames / self.audio_rate as f64 * HNS) as i64;
        unsafe {
            let bytes = std::slice::from_raw_parts(pcm.as_ptr() as *const u8, pcm.len() * 2);
            let s = Self::sample(bytes, t_s, dur)?;
            self.writer.WriteSample(idx, &s).map_err(err)
        }
    }

    pub fn finish(self) -> Result<(), String> {
        unsafe { self.writer.Finalize().map_err(err)? };
        drop(self.writer);
        if self.fast_start {
            faststart(std::path::Path::new(&self.path))?;
        }
        Ok(())
    }
}

// ------------------------------------------------------------------ fast start

/// Top-level MP4 boxes: (kind, start, total size).
fn top_boxes(f: &mut std::fs::File, len: u64) -> std::io::Result<Vec<([u8; 4], u64, u64)>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut out = Vec::new();
    let mut at = 0u64;
    while at + 8 <= len {
        f.seek(SeekFrom::Start(at))?;
        let mut h = [0u8; 16];
        f.read_exact(&mut h[..8])?;
        let mut size = u32::from_be_bytes(h[..4].try_into().unwrap()) as u64;
        if size == 1 {
            f.read_exact(&mut h[8..])?;
            size = u64::from_be_bytes(h[8..].try_into().unwrap());
        } else if size == 0 {
            size = len - at;
        }
        if size < 8 {
            break;
        }
        out.push((h[4..8].try_into().unwrap(), at, size));
        at += size;
    }
    Ok(out)
}

/// Add `delta` to every chunk offset (stco / co64) inside a moov box's bytes.
fn shift_chunk_offsets(b: &mut [u8], delta: u64) -> Result<(), String> {
    let mut at = 0usize;
    while at + 8 <= b.len() {
        let size = u32::from_be_bytes(b[at..at + 4].try_into().unwrap()) as usize;
        let kind: [u8; 4] = b[at + 4..at + 8].try_into().unwrap();
        if size < 8 || at + size > b.len() {
            break;
        }
        let body = &mut b[at + 8..at + size];
        match &kind {
            b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" => shift_chunk_offsets(body, delta)?,
            b"stco" if body.len() >= 8 => {
                let n = u32::from_be_bytes(body[4..8].try_into().unwrap()) as usize;
                for e in body[8..].chunks_exact_mut(4).take(n) {
                    let v = u32::from_be_bytes(e.try_into().unwrap()) as u64 + delta;
                    let v = u32::try_from(v).map_err(|_| "file too large to optimize for web".to_string())?;
                    e.copy_from_slice(&v.to_be_bytes());
                }
            }
            b"co64" if body.len() >= 8 => {
                let n = u32::from_be_bytes(body[4..8].try_into().unwrap()) as usize;
                for e in body[8..].chunks_exact_mut(8).take(n) {
                    let v = u64::from_be_bytes(e.try_into().unwrap()) + delta;
                    e.copy_from_slice(&v.to_be_bytes());
                }
            }
            _ => {}
        }
        at += size;
    }
    Ok(())
}

/// Move the moov box in front of mdat (what "optimize for web" means), like
/// qt-faststart: the index is read once, chunk offsets are shifted by its size,
/// and the file is rewritten with the media data streamed through.
pub fn faststart(path: &std::path::Path) -> Result<(), String> {
    use std::io::{Read, Seek, SeekFrom, Write};
    let e = |e: std::io::Error| e.to_string();
    let mut src = std::fs::File::open(path).map_err(e)?;
    let len = src.metadata().map_err(e)?.len();
    let boxes = top_boxes(&mut src, len).map_err(e)?;
    let moov = boxes.iter().position(|b| &b.0 == b"moov").ok_or("no moov box")?;
    let mdat = boxes.iter().position(|b| &b.0 == b"mdat").ok_or("no mdat box")?;
    if moov < mdat {
        return Ok(()); // already fast-start
    }
    let (_, m_at, m_size) = boxes[moov];
    let mut moov_bytes = vec![0u8; m_size as usize];
    src.seek(SeekFrom::Start(m_at)).map_err(e)?;
    src.read_exact(&mut moov_bytes).map_err(e)?;
    if u32::from_be_bytes(moov_bytes[..4].try_into().unwrap()) == 1 {
        return Err("unexpected 64-bit moov box".into());
    }
    shift_chunk_offsets(&mut moov_bytes[8..], m_size)?;

    let tmp = path.with_extension("faststart.tmp");
    {
        let mut dst = std::io::BufWriter::new(std::fs::File::create(&tmp).map_err(e)?);
        for (i, &(_, at, size)) in boxes.iter().enumerate() {
            if i == moov {
                continue;
            }
            if i == mdat {
                dst.write_all(&moov_bytes).map_err(e)?;
            }
            src.seek(SeekFrom::Start(at)).map_err(e)?;
            std::io::copy(&mut (&mut src).take(size), &mut dst).map_err(e)?;
        }
        dst.flush().map_err(e)?;
    }
    drop(src);
    std::fs::rename(&tmp, path).map_err(e)
}

// ------------------------------------------------------------------ colour

/// Packed BGRA to NV12 (BT.709, limited range), chroma averaged over 2x2 blocks.
/// `w` and `h` must be even.
pub fn bgra_to_nv12(src: &[u8], w: usize, h: usize) -> Vec<u8> {
    use rayon::prelude::*;
    let mut out = vec![0u8; w * h * 3 / 2];
    let (luma, chroma) = out.split_at_mut(w * h);
    let y_of = |p: &[u8]| -> u8 {
        let (b, g, r) = (p[0] as i32, p[1] as i32, p[2] as i32);
        (((47 * r + 157 * g + 16 * b + 128) >> 8) + 16) as u8
    };
    // two luma rows and their chroma row per job
    luma.par_chunks_mut(w * 2).zip(chroma.par_chunks_mut(w)).enumerate().for_each(|(pair, (ys, uv))| {
        let r0 = &src[pair * 2 * w * 4..(pair * 2 + 1) * w * 4];
        let r1 = &src[(pair * 2 + 1) * w * 4..(pair * 2 + 2) * w * 4];
        for x in 0..w {
            ys[x] = y_of(&r0[x * 4..]);
            ys[w + x] = y_of(&r1[x * 4..]);
        }
        for cx in 0..w / 2 {
            let (mut b, mut g, mut r) = (0i32, 0i32, 0i32);
            for p in [&r0[cx * 8..], &r0[cx * 8 + 4..], &r1[cx * 8..], &r1[cx * 8 + 4..]] {
                b += p[0] as i32;
                g += p[1] as i32;
                r += p[2] as i32;
            }
            let (b, g, r) = (b / 4, g / 4, r / 4);
            uv[cx * 2] = (((-26 * r - 87 * g + 112 * b + 128) >> 8) + 128).clamp(16, 240) as u8;
            uv[cx * 2 + 1] = (((112 * r - 102 * g - 10 * b + 128) >> 8) + 128).clamp(16, 240) as u8;
        }
    });
    out
}

// ------------------------------------------------------------------ scaling

/// Bilinear resize of packed BGRA (box-like for 2:1 and smaller ratios: samples
/// land between source pixels, so each output averages a 2x2 neighbourhood).
pub fn resize_bgra(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<u8> {
    use rayon::prelude::*;
    if sw == dw && sh == dh {
        return src.to_vec();
    }
    let mut out = vec![0u8; dw * dh * 4];
    if sw == 0 || sh == 0 {
        return out;
    }
    let fx = sw as f32 / dw as f32;
    let fy = sh as f32 / dh as f32;
    // precompute horizontal taps
    let xs: Vec<(usize, usize, u32)> = (0..dw)
        .map(|x| {
            let sx = ((x as f32 + 0.5) * fx - 0.5).max(0.0);
            let x0 = (sx as usize).min(sw - 1);
            let x1 = (x0 + 1).min(sw - 1);
            (x0 * 4, x1 * 4, ((sx - x0 as f32) * 256.0) as u32)
        })
        .collect();
    out.par_chunks_mut(dw * 4).enumerate().for_each(|(y, row)| {
        let sy = ((y as f32 + 0.5) * fy - 0.5).max(0.0);
        let y0 = (sy as usize).min(sh - 1);
        let y1 = (y0 + 1).min(sh - 1);
        let wy = ((sy - y0 as f32) * 256.0) as u32;
        let r0 = &src[y0 * sw * 4..(y0 + 1) * sw * 4];
        let r1 = &src[y1 * sw * 4..(y1 + 1) * sw * 4];
        for (x, &(a, b, wx)) in xs.iter().enumerate() {
            for c in 0..4 {
                let top = r0[a + c] as u32 * (256 - wx) + r0[b + c] as u32 * wx;
                let bot = r1[a + c] as u32 * (256 - wx) + r1[b + c] as u32 * wx;
                row[x * 4 + c] = ((top * (256 - wy) + bot * wy) >> 16) as u8;
            }
        }
    });
    out
}
