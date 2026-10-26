use anyhow::{Context, Result, ensure};
use gpui_kit::{prelude::*, *};
use objc2::msg_send;
use objc2_core_image::{CIContext, CIFilter, kCIFormatRGBA8};
use objc2_foundation::{NSData, ns_string};
use std::{ptr::NonNull, sync::Arc};

pub struct QrCode {
    width: usize,
    pixels: Vec<u8>,
}

impl QrCode {
    pub fn new(link: &str) -> Result<Self> {
        // SAFETY: QRCodeGenerator returns Apple's QR filter, which implements these
        // setters. The owned RGBA buffer stays valid throughout synchronous rendering.
        unsafe {
            let filter = CIFilter::QRCodeGenerator();
            let data = NSData::with_bytes(link.as_bytes());
            let _: () = msg_send![&filter, setMessage: &*data];
            let _: () = msg_send![&filter, setCorrectionLevel: ns_string!("M")];
            let image = filter
                .outputImage()
                .context("QR generator returned no image")?;
            let bounds = image.extent();
            ensure!(
                bounds.size.width > 0.0
                    && bounds.size.width <= 177.0
                    && bounds.size.width == bounds.size.height,
                "Invalid QR dimensions"
            );
            let width = bounds.size.width as usize;
            let mut pixels = vec![0_u8; width * width * 4];
            let pointer = NonNull::new(pixels.as_mut_ptr()).context("Empty QR buffer")?;
            CIContext::context().render_toBitmap_rowBytes_bounds_format_colorSpace(
                &image,
                pointer.cast(),
                (width * 4) as isize,
                bounds,
                kCIFormatRGBA8,
                None,
            );
            Ok(Self { width, pixels })
        }
    }
}

pub fn view(qr: Arc<QrCode>) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            window.paint_quad(fill(bounds, rgb(0xffffff)));
            let module = (bounds.size.width / (qr.width + 8) as f32).floor();
            let inset = (bounds.size.width - module * qr.width as f32) / 2.;
            for y in 0..qr.width {
                for x in 0..qr.width {
                    // Core Image's bitmap origin is bottom-left; GPUI's is top-left.
                    let offset = ((qr.width - 1 - y) * qr.width + x) * 4;
                    if qr.pixels[offset] < 128 {
                        let origin = bounds.origin
                            + point(inset + module * x as f32, inset + module * y as f32);
                        window.paint_quad(fill(
                            Bounds::new(origin, size(module, module)),
                            rgb(0x000000),
                        ));
                    }
                }
            }
        },
    )
    .size(px(260.))
}
