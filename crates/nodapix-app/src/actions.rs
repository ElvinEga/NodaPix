//! Editor actions. Dispatched from the macOS menu bar and from key bindings.

use gpui::actions;

actions!(
    nodapix,
    [
        Quit,
        Open,
        Export,
        Undo,
        Redo,
        ToggleCompare,
        ZoomIn,
        ZoomOut,
        Fit,
        ActualSize,
        RotateCW,
        RotateCCW,
        FlipH,
        FlipV,
        ResetAll,
        ResetAdjustments,
        ToggleSidebar,
        ToggleCrop,
        ApplyCrop,
        CancelCrop,
        ResetCrop,
    ]
);
