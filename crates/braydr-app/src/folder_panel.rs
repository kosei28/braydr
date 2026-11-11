//! The folder picker, opened where the field points. gpui's `prompt_for_paths` cannot say where
//! the panel starts, so this opens AppKit's panel itself.

use std::cell::Cell;
use std::path::{Path, PathBuf};

use block2::RcBlock;
use futures::channel::oneshot;
use objc2::rc::Retained;
use objc2::{MainThreadMarker, class, msg_send};
use objc2_app_kit::{NSModalResponse, NSModalResponseOK, NSOpenPanel};
use objc2_foundation::{NSString, NSURL};

/// Shows the panel inside `start`, so choosing at once keeps it. Resolves to `None` when the
/// user cancels or the panel cannot open. Call it on the main thread.
pub fn choose_directory(start: &Path) -> oneshot::Receiver<Option<PathBuf>> {
    let (done, chosen) = oneshot::channel();
    if MainThreadMarker::new().is_none() {
        return chosen;
    }
    // AppKit can return nil despite the nonnull annotation, as gpui notes for the same call.
    let panel: Option<Retained<NSOpenPanel>> = unsafe { msg_send![class!(NSOpenPanel), openPanel] };
    let Some(panel) = panel else {
        return chosen;
    };
    panel.setCanChooseDirectories(true);
    panel.setCanChooseFiles(false);
    panel.setAllowsMultipleSelection(false);
    panel.setCanCreateDirectories(true);
    panel.setResolvesAliases(false);
    if start.is_dir() {
        let path = NSString::from_str(&start.to_string_lossy());
        let url = NSURL::fileURLWithPath_isDirectory(&path, true);
        panel.setDirectoryURL(Some(&url));
    }
    let done = Cell::new(Some(done));
    let handler = RcBlock::new({
        let panel = panel.clone();
        move |response: NSModalResponse| {
            let Some(done) = done.take() else {
                return;
            };
            let path = (response == NSModalResponseOK)
                .then(|| panel.URL())
                .flatten()
                .and_then(|url| url.to_file_path());
            let _ = done.send(path);
        }
    });
    panel.beginWithCompletionHandler(&handler);
    chosen
}
