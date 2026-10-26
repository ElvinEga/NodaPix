# NodaPix Feature Roadmap & Pending Checklist

This checklist tracks all pending features from **Linux Mint Pix** (`/pix/`) being ported to **NodaPix** (Rust + GPUI). Each item will be checked off (`[x]`) as it is implemented and verified.

---

## 1. Interactive & On-Canvas Editing Tools

- [ ] **Horizon Line Straighten Tool**
  - *Pix reference*: [`pix/extensions/file_tools/gth-image-line-tool.c`](file:///Users/solution/Development/rea/pix/pix/extensions/file_tools/gth-image-line-tool.c)
  - *Description*: Click and drag a straight line directly across a crooked horizon, waterline, or architectural edge on the canvas. The angle is automatically calculated ($\theta = \text{atan2}(\Delta y, \Delta x)$) and applied to the straighten rotation engine.
  - *Target*: `crates/nodapix-app/src/canvas.rs` & `crates/nodapix-imaging/src/transform.rs`

- [ ] **Eyedropper / Color Picker & White Balance Sampler**
  - *Pix reference*: [`pix/extensions/file_tools/gth-file-tool-color-picker.c`](file:///Users/solution/Development/rea/pix/pix/extensions/file_tools/gth-file-tool-color-picker.c)
  - *Description*: Canvas interactive loupe/sampler showing RGB (0–255), HEX, and sRGB float values under the cursor. Clicking in "White Balance" mode samples a neutral gray patch and automatically calculates Kelvin temperature and tint offsets.
  - *Target*: `crates/nodapix-app/src/canvas.rs` & `crates/nodapix-imaging/src/color.rs`

- [ ] **Red-Eye Removal Tool**
  - *Pix reference*: [`pix/extensions/red_eye_removal/gth-file-tool-red-eye.c`](file:///Users/solution/Development/rea/pix/pix/extensions/red_eye_removal/gth-file-tool-red-eye.c)
  - *Description*: Click or drag a circular selection over flash pupil reflections. Pixels within the circle exceeding the red-ratio threshold ($R > 1.2 \times \max(G, B)$) are desaturated and darkened while preserving pupil specular highlights.
  - *Target*: `crates/nodapix-imaging/src/color.rs` & `crates/nodapix-app/src/canvas.rs`

- [ ] **Extended Crop Composition Guides**
  - *Pix reference*: [`pix/extensions/file_tools/gth-file-tool-crop.c`](file:///Users/solution/Development/rea/pix/pix/extensions/file_tools/gth-file-tool-crop.c)
  - *Description*: In addition to the existing Rule of Thirds grid, provide toggleable overlays for:
    - Golden Ratio / Golden Spiral
    - Center crosshairs & diagonal harmony lines
  - *Target*: `crates/nodapix-app/src/canvas.rs`

---

## 2. Advanced Color & Tonal Manipulation

- [ ] **Interactive Spline Tone Curves Editor**
  - *Pix reference*: [`pix/extensions/file_tools/gth-curve-editor.c`](file:///Users/solution/Development/rea/pix/pix/extensions/file_tools/gth-curve-editor.c), [`gth-file-tool-curves.c`](file:///Users/solution/Development/rea/pix/pix/extensions/file_tools/gth-file-tool-curves.c)
  - *Description*: A visual 256×256 tonal curve canvas supporting natural cubic spline interpolation with draggable control points. Supports:
    - Composite RGB curve (contrast and tone mapping)
    - Per-channel Red, Green, and Blue curves for color grading
    - Underlying live histogram backdrop
  - *Target*: `crates/nodapix-imaging/src/color.rs` & `crates/nodapix-app/src/app.rs`

- [ ] **Grayscale & Channel Mixer**
  - *Pix reference*: [`pix/extensions/file_tools/gth-file-tool-grayscale.c`](file:///Users/solution/Development/rea/pix/pix/extensions/file_tools/gth-file-tool-grayscale.c)
  - *Description*: Dedicated B&W conversion subpanel with selectable algorithms:
    - ITU-R BT.709 Luminance ($Y = 0.2126 R + 0.7152 G + 0.0722 B$)
    - Desaturate ($\frac{\max(R,G,B) + \min(R,G,B)}{2}$)
    - Custom 3-channel mixer sliders ($W_R + W_G + W_B = 1.0$)
  - *Target*: `crates/nodapix-imaging/src/color.rs` & `crates/nodapix-app/src/app.rs`

- [ ] **Negative / Color Inversion**
  - *Pix reference*: [`pix/extensions/file_tools/gth-file-tool-negative.c`](file:///Users/solution/Development/rea/pix/pix/extensions/file_tools/gth-file-tool-negative.c)
  - *Description*: 1-click photo negative conversion ($C_{\text{out}} = 1.0 - C_{\text{in}}$) with undo/redo support.
  - *Target*: `crates/nodapix-core/src/state.rs` & `crates/nodapix-imaging/src/color.rs`

- [ ] **Extended Special Effects Filter Suite**
  - *Pix reference*: [`pix/extensions/file_tools/cairo-effects.c`](file:///Users/solution/Development/rea/pix/pix/extensions/file_tools/cairo-effects.c)
  - *Description*: Implement Pix's remaining creative photo filters:
    - *Lomo*: high-saturation boost, heavy vignette, aggressive S-curve contrast
    - *Vintage*: lifted blacks, faded highlights, warm-yellow color tint
    - *Blurred Edges*: radial Gaussian blur mask leaving the center subject sharp
    - *Cross-Process*: cyan-shifted shadows and yellow-green highlights
  - *Target*: `crates/nodapix-imaging/src/color.rs` & `crates/nodapix-app/src/app.rs`

---

## 3. Viewer, Presentation & Color Management

- [ ] **Fullscreen Presentation Mode**
  - *Pix reference*: [`pix/extensions/image_viewer/gth-image-viewer-page.c`](file:///Users/solution/Development/rea/pix/pix/extensions/image_viewer/gth-image-viewer-page.c)
  - *Description*: Borderless full-screen display (`F11` / `Cmd+Ctrl+F`) with pitch-black backdrop, auto-hiding window chrome/status bars, and a floating translucent navigation HUD on mouse movement.
  - *Target*: `crates/nodapix-app/src/app.rs`

- [ ] **Automated Slideshow**
  - *Pix reference*: [`pix/extensions/slideshow/`](file:///Users/solution/Development/rea/pix/pix/extensions/slideshow/)
  - *Description*: Auto-advancing photo slideshow across current folder or selected images with configurable interval (1–30s), loop toggle, and Spacebar pause/resume.
  - *Target*: `crates/nodapix-app/src/app.rs`

- [ ] **Color Profile Management (ICC Profiles)**
  - *Pix reference*: [`pix/pix/gth-color-manager.c`](file:///Users/solution/Development/rea/pix/pix/pix/gth-color-manager.c)
  - *Description*: Read embedded ICC color profiles from JPEG, PNG, TIFF, and HEIC files (e.g. Display P3, Adobe RGB) and transform them accurately into the current macOS monitor's color space.
  - *Target*: `crates/nodapix-imaging/src/backend.rs`

---

## 4. Batch Operations & File Utilities

- [ ] **Batch Format Converter**
  - *Pix reference*: [`pix/extensions/convert_format/`](file:///Users/solution/Development/rea/pix/pix/extensions/convert_format/)
  - *Description*: Multi-file batch format conversion dialog allowing selection of multiple images, destination directory, output format (JPEG, PNG, WebP, TIFF), and quality slider.
  - *Target*: `crates/nodapix-app/src/app.rs`

- [ ] **Batch Series Renaming**
  - *Pix reference*: [`pix/extensions/rename_series/`](file:///Users/solution/Development/rea/pix/pix/extensions/rename_series/)
  - *Description*: Pattern-based batch file renamer with token replacement (e.g. `Trip_%04d`, `%Y%m%d_%H%M%S`, original name, counter start/step) with live preview table.
  - *Target*: `crates/nodapix-app/src/app.rs`

- [ ] **Batch EXIF Date/Time Adjuster**
  - *Pix reference*: [`pix/extensions/change_date/`](file:///Users/solution/Development/rea/pix/pix/extensions/change_date/)
  - *Description*: Batch adjust capture timestamps in EXIF blocks across multiple photos (e.g. offset by +N hours for timezone correction or synchronize camera clocks).
  - *Target*: `crates/nodapix-imaging/src/metadata.rs` & `crates/nodapix-app/src/app.rs`

- [ ] **Duplicate Image Finder**
  - *Pix reference*: [`pix/extensions/find_duplicates/`](file:///Users/solution/Development/rea/pix/pix/extensions/find_duplicates/)
  - *Description*: Fast folder scanner calculating 64-bit perceptual image hashes (dHash/pHash) and exact checksums to identify duplicate or near-duplicate photos.
  - *Target*: `crates/nodapix-app/src/`

- [ ] **Contact Sheet Generator**
  - *Pix reference*: [`pix/extensions/contact_sheet/`](file:///Users/solution/Development/rea/pix/pix/extensions/contact_sheet/)
  - *Description*: Generate a multi-row/multi-column photo thumbnail contact sheet exported as a single high-resolution image or PDF with optional file names and metadata captions.
  - *Target*: `crates/nodapix-imaging/` & `crates/nodapix-app/`

- [ ] **Native macOS Print Sheet**
  - *Pix reference*: [`pix/extensions/image_print/`](file:///Users/solution/Development/rea/pix/pix/extensions/image_print/)
  - *Description*: Standard macOS Cocoa printing sheet integration via AppKit `NSPrintOperation` supporting paper size, margins, scaling (fit/fill/100%), and print preview.
  - *Target*: `crates/nodapix-app/`

---

## 5. Completed Core Features

- [x] **Metal GPU Canvas**: Fast pan, zoom, pinch-to-zoom, 100% 1:1 view, and fit-to-window scaling.
- [x] **Compare Mode (`Shift+\`)**: Instant visual comparison between edited preview and original image.
- [x] **Orthogonal Transforms**: Lossless 90° CW, 90° CCW, Horizontal Flip, Vertical Flip.
- [x] **Basic Color Adjustments**: Exposure (EV), Brightness, Contrast, Saturation, Temperature (Kelvin), Tint, Shadows, Highlights.
- [x] **Sharpening (USM)**: 3-pass box blur unsharp mask with threshold clamp.
- [x] **Straighten & Auto-Crop**: -45° to +45° fine angle rotator with trigonometric inscribed crop.
- [x] **Interactive Crop Tool**: 8 draggable handles, aspect ratio presets (Free, Square, 3:2, 4:3, 16:9, Original), Rule of Thirds grid, spinboxes.
- [x] **Resize Image Dialog**: Pixel/percentage dimensions, aspect ratio locking, format and destination folder selection.
- [x] **Live Histogram**: 256-bin real-time luminance + RGB overlay widget.
- [x] **Auto Levels / Contrast**: 0.5% dynamic range histogram stretch.
- [x] **Vignette & Basic Filters**: Radial vignette falloff and B&W, Sepia, Warm Gold, Cool Nordic, Vivid presets.
- [x] **Folder Thumbnail Carousel (`F8`)**: Bottom filmstrip carousel with asynchronous background thumbnail loading.
- [x] **Right Sidebar Tools & Subpanels**: Pix/gthumb grid-style icon buttons; dedicated subpanels for Light, Balance, Levels, Hue/Sat, Sharpen, Vignette, Straighten.
- [x] **Full Lucide Vector Icon Set**: 35 resolution-independent Lucide SVG vector icons replacing all emojis and unicode glyphs.
- [x] **EXIF Inspection & Preservation**: In-app EXIF inspector modal; verbatim raw camera EXIF block preservation with normalized orientation on export.
- [x] **Apple Image I/O Framework**: Native Apple Silicon decoder support for iPhone HEIC/HEIF and ProRAW photos.
- [x] **macOS App Bundle**: Standalone signed `NodaPix.app` with Retina `.icns` and `Info.plist`.
