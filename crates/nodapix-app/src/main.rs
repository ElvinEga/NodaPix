//! NodaPix: native macOS image editor.

mod actions;
mod app;
mod canvas;
mod panel;
mod render;

use std::path::PathBuf;

use gpui::{
    App, AppContext, Bounds, Focusable, KeyBinding, Menu, MenuItem, SystemMenuType, WindowBounds,
    WindowOptions, px, size,
};

use crate::actions::{
    ActualSize, ApplyCrop, AutoEnhance, CancelCrop, Export, Fit, FlipH, FlipV, Open, Quit, Redo,
    ResetAdjustments, ResetAll, ResetCrop, RotateCCW, RotateCW, ToggleCompare, ToggleCrop,
    ToggleInfo, ToggleSidebar, Undo, ZoomIn, ZoomOut,
};
use crate::app::{Editor, KEY_CONTEXT};

fn main() {
    // GPUI's backends are separate crates at this revision; macOS must be chosen explicitly.
    let platform = std::rc::Rc::new(gpui_macos::MacPlatform::new(false));
    gpui::Application::with_platform(platform).run(|cx: &mut App| {
        let editor = cx.new(|cx| Editor::new(cx));

        cx.on_action(|_: &Quit, cx| cx.quit());

        cx.bind_keys([
            KeyBinding::new("cmd-o", Open, None),
            KeyBinding::new("cmd-z", Undo, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-shift-z", Redo, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-+", ZoomIn, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd--", ZoomOut, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-0", Fit, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-1", ActualSize, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-[", RotateCCW, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-]", RotateCW, Some(KEY_CONTEXT)),
            KeyBinding::new("shift-h", FlipH, Some(KEY_CONTEXT)),
            KeyBinding::new("shift-v", FlipV, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-e", ToggleSidebar, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-shift-s", Export, Some(KEY_CONTEXT)),
            KeyBinding::new("c", ToggleCrop, Some(KEY_CONTEXT)),
            KeyBinding::new("enter", ApplyCrop, Some(KEY_CONTEXT)),
            KeyBinding::new("escape", CancelCrop, Some(KEY_CONTEXT)),
            KeyBinding::new("shift-\\", ToggleCompare, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-backspace", ResetAll, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-i", ToggleInfo, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-u", AutoEnhance, Some(KEY_CONTEXT)),
        ]);

        cx.set_menus(vec![
            Menu::new("NodaPix").items(vec![
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Quit NodaPix", Quit),
            ]),
            Menu::new("File").items(vec![
                MenuItem::action("Open…", Open),
                MenuItem::action("Export…", Export),
                MenuItem::separator(),
                MenuItem::action("File Info…", ToggleInfo),
            ]),
            Menu::new("Edit").items(vec![
                MenuItem::action("Undo", Undo),
                MenuItem::action("Redo", Redo),
                MenuItem::separator(),
                MenuItem::action("Reset Adjustments", ResetAdjustments),
                MenuItem::action("Reset All", ResetAll),
            ]),
            Menu::new("View").items(vec![
                MenuItem::action("Zoom In", ZoomIn),
                MenuItem::action("Zoom Out", ZoomOut),
                MenuItem::action("Fit to Window", Fit),
                MenuItem::action("Actual Size", ActualSize),
                MenuItem::separator(),
                MenuItem::action("Show Original", ToggleCompare),
                MenuItem::action("Toggle Adjustments Panel", ToggleSidebar),
            ]),
            Menu::new("Image").items(vec![
                MenuItem::action("Auto Enhance", AutoEnhance),
                MenuItem::separator(),
                MenuItem::action("Crop", ToggleCrop),
                MenuItem::action("Reset Crop", ResetCrop),
                MenuItem::separator(),
                MenuItem::action("Rotate Left", RotateCCW),
                MenuItem::action("Rotate Right", RotateCW),
                MenuItem::action("Flip Horizontal", FlipH),
                MenuItem::action("Flip Vertical", FlipV),
            ]),
        ]);

        let bounds = Bounds::centered(None, size(px(1240.0), px(820.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            {
                let editor = editor.clone();
                move |window, cx| {
                    window.focus(&editor.focus_handle(cx), cx);
                    editor
                }
            },
        )
        .expect("failed to open the editor window");

        if let Some(path) = std::env::args().nth(1) {
            editor.update(cx, |editor, cx| editor.load(PathBuf::from(path), cx));
        }

        cx.activate(true);
    });
}
