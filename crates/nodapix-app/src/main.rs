//! NodaPix: native macOS image editor.

mod actions;
mod app;
mod batch;
mod canvas;
mod icon;
mod panel;
mod print;
mod render;

use std::path::PathBuf;

use async_channel::unbounded;
use gpui::{
    App, AppContext, Bounds, Focusable, KeyBinding, Menu, MenuItem, SystemMenuType, TitlebarOptions,
    WindowBounds, WindowOptions, px, size,
};

use crate::actions::{
    ActualSize, ApplyCrop, AutoEnhance, CancelCrop, Export, Fit, FlipH, FlipV, NextImage, Open,
    PrevImage, PrintImage, Quit, Redo, ResetAdjustments, ResetAll, ResetCrop, RotateCCW, RotateCW,
    Save, SaveAs, ToggleBatchConvert, ToggleBatchDate, ToggleBatchRename, ToggleCompare,
    ToggleContactSheet, ToggleCrop, ToggleFindDuplicates, ToggleFullscreen, ToggleInfo,
    ToggleResize, ToggleSidebar, ToggleSlideshow, ToggleThumbnailPane, Undo, ZoomIn, ZoomOut,
};
use crate::app::{Editor, KEY_CONTEXT};

fn url_to_path(raw: &str) -> Option<PathBuf> {
    if let Ok(url) = url::Url::parse(raw) {
        if url.scheme() == "file" {
            if let Ok(path) = url.to_file_path() {
                return Some(path);
            }
        }
    }
    let p = PathBuf::from(raw);
    if p.exists() {
        return Some(p);
    }
    None
}

fn main() {
    // GPUI's backends are separate crates at this revision; macOS must be chosen explicitly.
    let platform = std::rc::Rc::new(gpui_macos::MacPlatform::new(false));
    let app = gpui::Application::with_platform(platform);

    let (open_tx, open_rx) = unbounded::<PathBuf>();
    let tx_for_open = open_tx.clone();
    app.on_open_urls(move |urls| {
        for u in urls {
            if let Some(path) = url_to_path(&u) {
                let _ = tx_for_open.send_blocking(path);
            }
        }
    });

    app.run(move |cx: &mut App| {
        let editor = cx.new(|cx| Editor::new(cx));

        cx.on_action(|_: &Quit, cx| cx.quit());

        cx.bind_keys([
            KeyBinding::new("cmd-o", Open, None),
            KeyBinding::new("cmd-s", Save, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-shift-s", Export, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-alt-s", SaveAs, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-p", PrintImage, Some(KEY_CONTEXT)),
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
            KeyBinding::new("c", ToggleCrop, Some(KEY_CONTEXT)),
            KeyBinding::new("enter", ApplyCrop, Some(KEY_CONTEXT)),
            KeyBinding::new("escape", CancelCrop, Some(KEY_CONTEXT)),
            KeyBinding::new("shift-\\", ToggleCompare, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-backspace", ResetAll, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-i", ToggleInfo, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-u", AutoEnhance, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-r", ToggleResize, Some(KEY_CONTEXT)),
            KeyBinding::new("f8", ToggleThumbnailPane, Some(KEY_CONTEXT)),
            KeyBinding::new("f11", ToggleFullscreen, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-ctrl-f", ToggleFullscreen, Some(KEY_CONTEXT)),
            KeyBinding::new("f", ToggleFullscreen, Some(KEY_CONTEXT)),
            KeyBinding::new("space", ToggleSlideshow, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-shift-c", ToggleBatchConvert, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-shift-r", ToggleBatchRename, Some(KEY_CONTEXT)),
            KeyBinding::new("cmd-shift-d", ToggleFindDuplicates, Some(KEY_CONTEXT)),
            KeyBinding::new("left", PrevImage, Some(KEY_CONTEXT)),
            KeyBinding::new("right", NextImage, Some(KEY_CONTEXT)),
        ]);

        cx.set_menus(vec![
            Menu::new("NodaPix").items(vec![
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Quit NodaPix", Quit),
            ]),
            Menu::new("File").items(vec![
                MenuItem::action("Open…", Open),
                MenuItem::separator(),
                MenuItem::action("Save", Save),
                MenuItem::action("Save As…", SaveAs),
                MenuItem::action("Export…", Export),
                MenuItem::separator(),
                MenuItem::action("File Info…", ToggleInfo),
                MenuItem::separator(),
                MenuItem::action("Print…", PrintImage),
            ]),
            Menu::new("Edit").items(vec![
                MenuItem::action("Undo", Undo),
                MenuItem::action("Redo", Redo),
                MenuItem::separator(),
                MenuItem::action("Reset Adjustments", ResetAdjustments),
                MenuItem::action("Reset All", ResetAll),
            ]),
            Menu::new("View").items(vec![
                MenuItem::action("Previous Image", PrevImage),
                MenuItem::action("Next Image", NextImage),
                MenuItem::separator(),
                MenuItem::action("Zoom In", ZoomIn),
                MenuItem::action("Zoom Out", ZoomOut),
                MenuItem::action("Fit to Window", Fit),
                MenuItem::action("Actual Size", ActualSize),
                MenuItem::separator(),
                MenuItem::action("Show Original", ToggleCompare),
                MenuItem::action("Toggle Adjustments Panel", ToggleSidebar),
                MenuItem::action("Toggle Thumbnail Pane", ToggleThumbnailPane),
                MenuItem::separator(),
                MenuItem::action("Fullscreen Presentation", ToggleFullscreen),
                MenuItem::action("Play / Pause Slideshow", ToggleSlideshow),
            ]),
            Menu::new("Image").items(vec![
                MenuItem::action("Auto Enhance", AutoEnhance),
                MenuItem::separator(),
                MenuItem::action("Crop", ToggleCrop),
                MenuItem::action("Reset Crop", ResetCrop),
                MenuItem::action("Resize Images…", ToggleResize),
                MenuItem::separator(),
                MenuItem::action("Rotate Left", RotateCCW),
                MenuItem::action("Rotate Right", RotateCW),
                MenuItem::action("Flip Horizontal", FlipH),
                MenuItem::action("Flip Vertical", FlipV),
            ]),
            Menu::new("Tools").items(vec![
                MenuItem::action("Convert Format…", ToggleBatchConvert),
                MenuItem::action("Rename Series…", ToggleBatchRename),
                MenuItem::action("Adjust Date/Time…", ToggleBatchDate),
                MenuItem::separator(),
                MenuItem::action("Find Duplicate Images…", ToggleFindDuplicates),
                MenuItem::separator(),
                MenuItem::action("Contact Sheet Generator…", ToggleContactSheet),
            ]),
        ]);

        let bounds = Bounds::centered(None, size(px(1240.0), px(820.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("NodaPix".into()),
                    ..Default::default()
                }),
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

        let editor_for_open = editor.clone();
        cx.spawn(|cx: &mut gpui::AsyncApp| {
            let cx = cx.clone();
            async move {
                while let Ok(path) = open_rx.recv().await {
                    let _ = cx.update(|app| {
                        editor_for_open.update(app, |editor, cx| {
                            editor.open_path_with_save_prompt(path, cx);
                        });
                    });
                }
            }
        })
        .detach();

        if let Some(arg) = std::env::args().nth(1) {
            if !arg.starts_with("-psn") {
                let p = PathBuf::from(arg);
                if p.exists() {
                    let _ = open_tx.send_blocking(p);
                }
            }
        }

        cx.activate(true);
    });
}
