//! The image viewport: GPU-painted canvas with zoom, pan, and fit-to-window.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    canvas, div, fill, point, px, size, Bounds, Corners, ParentElement, Pixels, Point,
    RenderImage, Size, Styled, rgb,
};

/// One image pixel maps to one point at `Percent(1.0)`. Retina displays sample the
/// texture at 2x, so zoom stays in logical units and is display-independent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Zoom {
    Fit,
    Percent(f32),
}

/// Padding kept around the image when fitting, so it never sits flush with the chrome.
pub const FIT_MARGIN: f32 = 48.0;

pub const MIN_SCALE: f32 = 0.02;
pub const MAX_SCALE: f32 = 8.0;

pub fn fit_scale(image: Size<f32>, available: Size<f32>) -> f32 {
    if image.width <= 0.0 || image.height <= 0.0 {
        return 1.0;
    }
    let by_width = (available.width - FIT_MARGIN) / image.width;
    let by_height = (available.height - FIT_MARGIN) / image.height;
    by_width.min(by_height).clamp(MIN_SCALE, MAX_SCALE)
}

pub struct Viewport {
    pub texture: Option<Arc<RenderImage>>,
    pub image: Size<f32>,
    pub zoom: Zoom,
    pub pan: Point<Pixels>,
    pub backdrop: u32,
    pub frame: bool,
    /// Receives the canvas bounds each paint so input handlers can anchor zoom to
    /// the cursor without re-deriving the layout.
    pub bounds_out: Option<Rc<Cell<Option<Bounds<Pixels>>>>>,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            texture: None,
            image: size(1.0, 1.0),
            zoom: Zoom::Fit,
            pan: point(px(0.0), px(0.0)),
            backdrop: 0x1a1a_1c,
            frame: true,
            bounds_out: None,
        }
    }
}

impl Viewport {
    pub fn element(self) -> gpui::Div {
        let backdrop = rgb(self.backdrop);
        let frame_color = rgb(0x4a_4a_50);
        let image = self.image;
        let zoom = self.zoom;
        let pan = self.pan;
        let texture = self.texture;
        let draw_frame = self.frame;
        let bounds_out = self.bounds_out;

        div()
            .size_full()
            .relative()
            .overflow_hidden()
            .child(
                canvas(
                    move |_, _, _| {},
                    move |bounds, _, window, _| {
                        if let Some(bounds_out) = &bounds_out {
                            bounds_out.set(Some(bounds));
                        }

                        window.paint_quad(fill(bounds, backdrop));

                        let Some(texture) = texture.clone() else {
                            return;
                        };

                        let avail_w = f32::from(bounds.size.width);
                        let avail_h = f32::from(bounds.size.height);
                        let scale = match zoom {
                            Zoom::Percent(scale) => scale,
                            Zoom::Fit => fit_scale(image, size(avail_w, avail_h)),
                        };
                        let width = image.width * scale;
                        let height = image.height * scale;

                        // Pan is measured from the viewport centre, so a zoomed image
                        // stays anchored while resizing the window.
                        let origin_x = f32::from(bounds.origin.x);
                        let origin_y = f32::from(bounds.origin.y);
                        let pan_x = f32::from(pan.x);
                        let pan_y = f32::from(pan.y);
                        let centre_x = origin_x + avail_w / 2.0 + pan_x;
                        let centre_y = origin_y + avail_h / 2.0 + pan_y;
                        let dest = Bounds {
                            origin: point(px(centre_x - width / 2.0), px(centre_y - height / 2.0)),
                            size: size(px(width), px(height)),
                        };

                        let _ = window.paint_image(
                            dest,
                            bounds,
                            Corners::all(px(0.0)),
                            texture,
                            0,
                            false,
                        );

                        if draw_frame {
                            let hair = px(1.0);
                            window.paint_quad(fill(
                                Bounds {
                                    origin: point(dest.origin.x, dest.origin.y - hair),
                                    size: size(dest.size.width, hair),
                                },
                                frame_color,
                            ));
                            window.paint_quad(fill(
                                Bounds {
                                    origin: point(dest.origin.x, dest.origin.y + dest.size.height),
                                    size: size(dest.size.width, hair),
                                },
                                frame_color,
                            ));
                            window.paint_quad(fill(
                                Bounds {
                                    origin: point(dest.origin.x - hair, dest.origin.y),
                                    size: size(hair, dest.size.height),
                                },
                                frame_color,
                            ));
                            window.paint_quad(fill(
                                Bounds {
                                    origin: point(dest.origin.x + dest.size.width, dest.origin.y),
                                    size: size(hair, dest.size.height),
                                },
                                frame_color,
                            ));
                        }
                    },
                )
                .size_full(),
            )
    }
}
