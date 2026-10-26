//! Video stickers: WebM with VP9 color frames and a second VP9 stream for
//! transparency (Matroska BlockAdditional, ID 1). The container is read
//! here; frames are decoded by libvpx, linked statically (see build.rs).

use std::{ffi::c_void, ptr};

use anyhow::{Result, bail, ensure};

/// One video frame: its time in milliseconds, the color frame and, for
/// stickers with transparency, the alpha frame.
pub struct Packet {
    pub time_ms: f64,
    pub color: Vec<u8>,
    pub alpha: Option<Vec<u8>>,
}

const SEGMENT: u32 = 0x1853_8067;
const INFO: u32 = 0x1549_A966;
const TIMESTAMP_SCALE: u32 = 0x2A_D7B1;
const CLUSTER: u32 = 0x1F43_B675;
const CLUSTER_TIMESTAMP: u32 = 0xE7;
const SIMPLE_BLOCK: u32 = 0xA3;
const BLOCK_GROUP: u32 = 0xA0;
const BLOCK: u32 = 0xA1;
const BLOCK_ADDITIONS: u32 = 0x75A1;
const BLOCK_MORE: u32 = 0xA6;
const BLOCK_ADD_ID: u32 = 0xEE;
const BLOCK_ADDITIONAL: u32 = 0xA5;

/// The video frames of a WebM file, in order. Masters we care about are
/// walked into (so unknown sizes don't matter); everything else is skipped.
pub fn packets(data: &[u8]) -> Result<Vec<Packet>> {
    let mut packets: Vec<Packet> = Vec::new();
    let mut scale_ns = 1_000_000.0;
    let mut cluster_ms = 0.0;
    let mut track = None;
    let mut add_id = 1;
    let mut pos = 0;
    while pos < data.len() {
        let (id, id_len) = vint(&data[pos..], true)?;
        let (size, size_len) = vint(&data[pos + id_len..], false)?;
        let start = pos + id_len + size_len;
        // IDs keep their marker, so they're never "unknown".
        let id = id.unwrap_or(0) as u32;
        if matches!(
            id,
            SEGMENT | INFO | CLUSTER | BLOCK_GROUP | BLOCK_ADDITIONS | BLOCK_MORE
        ) {
            pos = start;
            continue;
        }
        let Some(size) = size else {
            bail!("unknown size outside a container");
        };
        let end = start + usize::try_from(size)?;
        ensure!(end <= data.len(), "element runs past the end");
        let body = &data[start..end];
        match id {
            TIMESTAMP_SCALE => scale_ns = uint(body) as f64,
            CLUSTER_TIMESTAMP => cluster_ms = uint(body) as f64 * scale_ns / 1e6,
            SIMPLE_BLOCK | BLOCK => {
                let (number, number_len) = vint(body, false)?;
                ensure!(body.len() >= number_len + 3, "short block");
                // Only the first track: video stickers have no audio.
                if *track.get_or_insert(number) == number {
                    let relative = i16::from_be_bytes([body[number_len], body[number_len + 1]]);
                    let flags = body[number_len + 2];
                    ensure!(flags & 0x06 == 0, "laced video blocks aren't supported");
                    packets.push(Packet {
                        time_ms: cluster_ms + f64::from(relative) * scale_ns / 1e6,
                        color: body[number_len + 3..].to_vec(),
                        alpha: None,
                    });
                }
            }
            BLOCK_ADD_ID => add_id = uint(body),
            BLOCK_ADDITIONAL => {
                // ID 1 is the alpha channel; it follows its Block.
                if add_id == 1
                    && let Some(packet) = packets.last_mut()
                {
                    packet.alpha = Some(body.to_vec());
                }
                add_id = 1;
            }
            _ => {}
        }
        pos = end;
    }
    Ok(packets)
}

/// An EBML variable-length integer. IDs keep their length marker; sizes
/// don't, and all-ones sizes mean "unknown".
fn vint(data: &[u8], keep_marker: bool) -> Result<(Option<u64>, usize)> {
    let first = *data.first().ok_or_else(|| anyhow::anyhow!("truncated"))?;
    let len = first.leading_zeros() as usize + 1;
    ensure!(len <= 8 && data.len() >= len, "bad variable-length integer");
    let mut value = if keep_marker {
        u64::from(first)
    } else {
        u64::from(first) & (0xFF_u64 >> len)
    };
    for &byte in &data[1..len] {
        value = (value << 8) | u64::from(byte);
    }
    let all_ones = (1u64 << (7 * len)) - 1;
    let unknown = !keep_marker && value == all_ones;
    Ok((if unknown { None } else { Some(value) }, len))
}

fn uint(body: &[u8]) -> u64 {
    body.iter()
        .fold(0, |value, &byte| (value << 8) | u64::from(byte))
}

// --- libvpx ----------------------------------------------------------------

/// `VPX_DECODER_ABI_VERSION` for libvpx 1.x: 3 + (4 + image ABI 5).
const VPX_DECODER_ABI_VERSION: i32 = 12;
const VPX_IMG_FMT_I420: i32 = 0x100 | 2;

#[repr(C)]
struct VpxImage {
    fmt: i32,
    cs: i32,
    range: i32,
    w: u32,
    h: u32,
    bit_depth: u32,
    d_w: u32,
    d_h: u32,
    r_w: u32,
    r_h: u32,
    x_chroma_shift: u32,
    y_chroma_shift: u32,
    planes: [*mut u8; 4],
    stride: [i32; 4],
    // Further fields are private to libvpx; images are only read here.
}

/// `vpx_codec_ctx_t`, opaque: 56 bytes on 64-bit, with room to spare.
#[repr(C, align(8))]
struct VpxContext([u8; 128]);

// build.rs copies libvpx.a under this name, so the linker can't pick a
// shared libvpx from the same directory.
#[link(name = "kitevpx", kind = "static")]
unsafe extern "C" {
    fn vpx_codec_vp9_dx() -> *const c_void;
    fn vpx_codec_dec_init_ver(
        ctx: *mut VpxContext,
        iface: *const c_void,
        cfg: *const c_void,
        flags: std::ffi::c_long,
        version: i32,
    ) -> i32;
    fn vpx_codec_decode(
        ctx: *mut VpxContext,
        data: *const u8,
        size: u32,
        user: *mut c_void,
        deadline: std::ffi::c_long,
    ) -> i32;
    fn vpx_codec_get_frame(ctx: *mut VpxContext, iter: *mut *const c_void) -> *const VpxImage;
    fn vpx_codec_destroy(ctx: *mut VpxContext) -> i32;
}

/// A VP9 decoder.
pub struct Decoder(Box<VpxContext>);

/// A decoded 8-bit 4:2:0 frame, planes copied out.
pub struct Yuv {
    pub width: usize,
    pub height: usize,
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
}

impl Decoder {
    pub fn new() -> Result<Self> {
        let mut ctx = Box::new(VpxContext([0; 128]));
        // SAFETY: Initializes the zeroed context in place with the VP9
        // interface, default configuration and this header's ABI version.
        let err = unsafe {
            vpx_codec_dec_init_ver(
                &mut *ctx,
                vpx_codec_vp9_dx(),
                ptr::null(),
                0,
                VPX_DECODER_ABI_VERSION,
            )
        };
        ensure!(err == 0, "libvpx couldn't start ({err})");
        Ok(Self(ctx))
    }

    /// Decodes one frame; None when it produces no displayable picture.
    pub fn decode(&mut self, data: &[u8]) -> Result<Option<Yuv>> {
        // SAFETY: The context is initialized; `data` outlives the call.
        let err = unsafe {
            vpx_codec_decode(
                &mut *self.0,
                data.as_ptr(),
                u32::try_from(data.len())?,
                ptr::null_mut(),
                0,
            )
        };
        ensure!(err == 0, "VP9 decoding failed ({err})");
        let mut iter = ptr::null();
        // SAFETY: Images returned by libvpx stay valid until the next
        // decode call; their planes are copied out before returning.
        unsafe {
            let image = vpx_codec_get_frame(&mut *self.0, &mut iter);
            let Some(image) = image.as_ref() else {
                return Ok(None);
            };
            ensure!(image.fmt == VPX_IMG_FMT_I420, "unsupported pixel format");
            let (width, height) = (image.d_w as usize, image.d_h as usize);
            let plane = |index: usize, w: usize, h: usize| {
                let stride = image.stride[index] as usize;
                let mut out = Vec::with_capacity(w * h);
                for row in 0..h {
                    out.extend_from_slice(std::slice::from_raw_parts(
                        image.planes[index].add(row * stride),
                        w,
                    ));
                }
                out
            };
            let (cw, ch) = (width.div_ceil(2), height.div_ceil(2));
            Ok(Some(Yuv {
                width,
                height,
                y: plane(0, width, height),
                u: plane(1, cw, ch),
                v: plane(2, cw, ch),
            }))
        }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        // SAFETY: Destroys the context initialized in `new`, once.
        unsafe {
            vpx_codec_destroy(&mut *self.0);
        }
    }
}

impl Yuv {
    /// BT.601 limited-range YUV to BGRA, with alpha from a decoded alpha
    /// frame's luma (opaque without one).
    pub fn to_bgra(&self, alpha: Option<&Yuv>) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.width * self.height * 4);
        let cw = self.width.div_ceil(2);
        for row in 0..self.height {
            for col in 0..self.width {
                let c = i32::from(self.y[row * self.width + col]) - 16;
                let chroma = (row / 2) * cw + col / 2;
                let d = i32::from(self.u[chroma]) - 128;
                let e = i32::from(self.v[chroma]) - 128;
                let clamp = |x: i32| ((x + 128) >> 8).clamp(0, 255) as u8;
                let a = alpha
                    .filter(|a| a.width == self.width && a.height == self.height)
                    .map_or(255, |a| a.y[row * self.width + col]);
                out.extend_from_slice(&[
                    clamp(298 * c + 516 * d),
                    clamp(298 * c - 100 * d - 208 * e),
                    clamp(298 * c + 409 * e),
                    a,
                ]);
            }
        }
        out
    }
}
