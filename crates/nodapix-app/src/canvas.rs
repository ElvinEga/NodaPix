//! The image viewport: GPU-painted canvas with zoom, pan, fit-to-window, and interactive crop overlay.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    canvas, div, fill, point, px, size, rgba, Bounds, Corners, ParentElement, Pixels, Point,
    RenderImage, Size, Styled, rgb,
};
use nodapix_imaging::CropRect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CropHandle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
    Inside,
}

pub fn hit_test_crop(
    cursor: Point<Pixels>,
    img_dest: Bounds<Pixels>,
    crop: CropRect,
) -> Option<CropHandle> {
    let img_ox = f32::from(img_dest.origin.x);
    let img_oy = f32::from(img_dest.origin.y);
    let img_w = f32::from(img_dest.size.width);
    let img_h = f32::from(img_dest.size.height);

    let cx = img_ox + crop.x * img_w;
    let cy = img_oy + crop.y * img_h;
    let cw = crop.width * img_w;
    let ch = crop.height * img_h;

    let cur_x = f32::from(cursor.x);
    let cur_y = f32::from(cursor.y);
    let dist_sq = |hx: f32, hy: f32| (cur_x - hx).powi(2) + (cur_y - hy).powi(2);
    let threshold = 16.0 * 16.0;

    // Corner handles prioritized
    if dist_sq(cx, cy) <= threshold {
        return Some(CropHandle::TopLeft);
    }
    if dist_sq(cx + cw, cy) <= threshold {
        return Some(CropHandle::TopRight);
    }
    if dist_sq(cx, cy + ch) <= threshold {
        return Some(CropHandle::BottomLeft);
    }
    if dist_sq(cx + cw, cy + ch) <= threshold {
        return Some(CropHandle::BottomRight);
    }

    // Edge handles
    if dist_sq(cx + cw / 2.0, cy) <= threshold {
        return Some(CropHandle::Top);
    }
    if dist_sq(cx + cw / 2.0, cy + ch) <= threshold {
        return Some(CropHandle::Bottom);
    }
    if dist_sq(cx, cy + ch / 2.0) <= threshold {
        return Some(CropHandle::Left);
    }
    if dist_sq(cx + cw, cy + ch / 2.0) <= threshold {
        return Some(CropHandle::Right);
    }

    // Inside box
    if cur_x >= cx && cur_x <= cx + cw && cur_y >= cy && cur_y <= cy + ch {
        return Some(CropHandle::Inside);
    }

    None
}

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
    pub crop: Option<CropRect>,
    pub crop_mode: bool,
    /// Receives the canvas bounds each paint so input handlers can anchor zoom to
    /// the cursor without re-deriving the layout.
    pub bounds_out: Option<Rc<Cell<Option<Bounds<Pixels>>>>>,
    /// Receives the image destination bounds on screen for crop handle hit testing.
    pub image_dest_out: Option<Rc<Cell<Option<Bounds<Pixels>>>>>,
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
            crop: None,
            crop_mode: false,
            bounds_out: None,
            image_dest_out: None,
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
        let crop_opt = self.crop;
        let crop_mode = self.crop_mode;
        let bounds_out = self.bounds_out;
        let image_dest_out = self.image_dest_out;

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

                        if let Some(dest_out) = &image_dest_out {
                            dest_out.set(Some(dest));
                        }

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

                        // Interactive Crop Overlay
                        if crop_mode {
                            let crop = crop_opt.unwrap_or(CropRect {
                                x: 0.0,
                                y: 0.0,
                                width: 1.0,
                                height: 1.0,
                            });

                            let img_ox = f32::from(dest.origin.x);
                            let img_oy = f32::from(dest.origin.y);

                            let cx = img_ox + crop.x * width;
                            let cy = img_oy + crop.y * height;
                            let cw = crop.width * width;
                            let ch = crop.height * height;

                            let scrim = rgba(0x00_00_00_a0);

                            // Top scrim
                            if cy > img_oy {
                                window.paint_quad(fill(
                                    Bounds {
                                        origin: point(px(img_ox), px(img_oy)),
                                        size: size(px(width), px(cy - img_oy)),
                                    },
                                    scrim,
                                ));
                            }
                            // Bottom scrim
                            if cy + ch < img_oy + height {
                                window.paint_quad(fill(
                                    Bounds {
                                        origin: point(px(img_ox), px(cy + ch)),
                                        size: size(px(width), px((img_oy + height) - (cy + ch))),
                                    },
                                    scrim,
                                ));
                            }
                            // Left scrim
                            if cx > img_ox {
                                window.paint_quad(fill(
                                    Bounds {
                                        origin: point(px(img_ox), px(cy)),
                                        size: size(px(cx - img_ox), px(ch)),
                                    },
                                    scrim,
                                ));
                            }
                            // Right scrim
                            if cx + cw < img_ox + width {
                                window.paint_quad(fill(
                                    Bounds {
                                        origin: point(px(cx + cw), px(cy)),
                                        size: size(px((img_ox + width) - (cx + cw)), px(ch)),
                                    },
                                    scrim,
                                ));
                            }

                            // Crop box border
                            let border_color = rgb(0xff_ff_ff);
                            let line = px(1.5);
                            window.paint_quad(fill(
                                Bounds {
                                    origin: point(px(cx), px(cy)),
                                    size: size(px(cw), line),
                                },
                                border_color,
                            ));
                            window.paint_quad(fill(
                                Bounds {
                                    origin: point(px(cx), px(cy + ch - 1.5)),
                                    size: size(px(cw), line),
                                },
                                border_color,
                            ));
                            window.paint_quad(fill(
                                Bounds {
                                    origin: point(px(cx), px(cy)),
                                    size: size(line, px(ch)),
                                },
                                border_color,
                            ));
                            window.paint_quad(fill(
                                Bounds {
                                    origin: point(px(cx + cw - 1.5), px(cy)),
                                    size: size(line, px(ch)),
                                },
                                border_color,
                            ));

                            // Rule-of-thirds grid
                            let grid_color = rgba(0xff_ff_ff_50);
                            let grid_line = px(1.0);
                            window.paint_quad(fill(
                                Bounds {
                                    origin: point(px(cx), px(cy + ch / 3.0)),
                                    size: size(px(cw), grid_line),
                                },
                                grid_color,
                            ));
                            window.paint_quad(fill(
                                Bounds {
                                    origin: point(px(cx), px(cy + 2.0 * ch / 3.0)),
                                    size: size(px(cw), grid_line),
                                },
                                grid_color,
                            ));
                            window.paint_quad(fill(
                                Bounds {
                                    origin: point(px(cx + cw / 3.0), px(cy)),
                                    size: size(grid_line, px(ch)),
                                },
                                grid_color,
                            ));
                            window.paint_quad(fill(
                                Bounds {
                                    origin: point(px(cx + 2.0 * cw / 3.0), px(cy)),
                                    size: size(grid_line, px(ch)),
                                },
                                grid_color,
                            ));

                            // 8 Handles
                            let handle_sz = 8.0;
                            let h_half = handle_sz / 2.0;
                            let handle_pts = [
                                (cx, cy),                          // TopLeft
                                (cx + cw / 2.0, cy),               // Top
                                (cx + cw, cy),                     // TopRight
                                (cx + cw, cy + ch / 2.0),          // Right
                                (cx + cw, cy + ch),                // BottomRight
                                (cx + cw / 2.0, cy + ch),          // Bottom
                                (cx, cy + ch),                     // BottomLeft
                                (cx, cy + ch / 2.0),               // Left
                            ];

                            for (hx, hy) in handle_pts {
                                window.paint_quad(fill(
                                    Bounds {
                                        origin: point(px(hx - h_half), px(hy - h_half)),
                                        size: size(px(handle_sz), px(handle_sz)),
                                    },
                                    border_color,
                                ));
                            }
                        }
                    },
                ),
            )
    }
}
