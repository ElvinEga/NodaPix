//! Lucide SVG vector icon integration for GPUI.
//!
//! Provides pixel-perfect, resolution-independent vector icons styled via GPUI text colors.

use gpui::{px, svg, Hsla, Styled, Svg};

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LucideIcon {
    Activity,
    Check,
    ChevronLeft,
    ChevronRight,
    Contrast,
    Crop,
    Download,
    Eye,
    Film,
    FlipHorizontal,
    FlipVertical,
    FolderOpen,
    Images,
    Info,
    Maximize2,
    Minimize2,
    Minus,
    Palette,
    Pipette,
    Plus,
    Redo2,
    RotateCcw,
    RotateCw,
    Save,
    Scaling,
    SlidersHorizontal,
    Sliders,
    Sparkles,
    Split,
    Sun,
    Undo2,
    Wand2,
    Wrench,
    X,
    Zap,
    ZoomIn,
}

impl LucideIcon {
    pub const fn svg_bytes(self) -> &'static [u8] {
        match self {
            Self::Activity => include_bytes!("../assets/icons/activity.svg"),
            Self::Check => include_bytes!("../assets/icons/check.svg"),
            Self::ChevronLeft => include_bytes!("../assets/icons/chevron-left.svg"),
            Self::ChevronRight => include_bytes!("../assets/icons/chevron-right.svg"),
            Self::Contrast => include_bytes!("../assets/icons/contrast.svg"),
            Self::Crop => include_bytes!("../assets/icons/crop.svg"),
            Self::Download => include_bytes!("../assets/icons/download.svg"),
            Self::Eye => include_bytes!("../assets/icons/eye.svg"),
            Self::Film => include_bytes!("../assets/icons/film.svg"),
            Self::FlipHorizontal => include_bytes!("../assets/icons/flip-horizontal.svg"),
            Self::FlipVertical => include_bytes!("../assets/icons/flip-vertical.svg"),
            Self::FolderOpen => include_bytes!("../assets/icons/folder-open.svg"),
            Self::Images => include_bytes!("../assets/icons/images.svg"),
            Self::Info => include_bytes!("../assets/icons/info.svg"),
            Self::Maximize2 => include_bytes!("../assets/icons/maximize-2.svg"),
            Self::Minimize2 => include_bytes!("../assets/icons/minimize-2.svg"),
            Self::Minus => include_bytes!("../assets/icons/minus.svg"),
            Self::Palette => include_bytes!("../assets/icons/palette.svg"),
            Self::Pipette => include_bytes!("../assets/icons/pipette.svg"),
            Self::Plus => include_bytes!("../assets/icons/plus.svg"),
            Self::Redo2 => include_bytes!("../assets/icons/redo-2.svg"),
            Self::RotateCcw => include_bytes!("../assets/icons/rotate-ccw.svg"),
            Self::RotateCw => include_bytes!("../assets/icons/rotate-cw.svg"),
            Self::Save => include_bytes!("../assets/icons/save.svg"),
            Self::Scaling => include_bytes!("../assets/icons/scaling.svg"),
            Self::SlidersHorizontal => include_bytes!("../assets/icons/sliders-horizontal.svg"),
            Self::Sliders => include_bytes!("../assets/icons/sliders.svg"),
            Self::Sparkles => include_bytes!("../assets/icons/sparkles.svg"),
            Self::Split => include_bytes!("../assets/icons/split.svg"),
            Self::Sun => include_bytes!("../assets/icons/sun.svg"),
            Self::Undo2 => include_bytes!("../assets/icons/undo-2.svg"),
            Self::Wand2 => include_bytes!("../assets/icons/wand-2.svg"),
            Self::Wrench => include_bytes!("../assets/icons/wrench.svg"),
            Self::X => include_bytes!("../assets/icons/x.svg"),
            Self::Zap => include_bytes!("../assets/icons/zap.svg"),
            Self::ZoomIn => include_bytes!("../assets/icons/zoom-in.svg"),
        }
    }

    /// Build a GPUI SVG element with explicit width/height and color.
    pub fn element(self, size: f32, color: impl Into<Hsla>) -> Svg {
        svg()
            .data(self.svg_bytes())
            .w(px(size))
            .h(px(size))
            .flex_shrink_0()
            .text_color(color.into())
    }

    /// Build a GPUI SVG element inheriting text_color from parent style context.
    #[allow(dead_code)]
    pub fn icon(self, size: f32) -> Svg {
        svg()
            .data(self.svg_bytes())
            .w(px(size))
            .h(px(size))
            .flex_shrink_0()
    }
}
