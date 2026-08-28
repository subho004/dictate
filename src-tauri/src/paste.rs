//! Paste-at-cursor and selection capture — both must run on the main thread (see
//! dictation.rs): arboard/enigo touch macOS AppKit/CoreGraphics APIs that can
//! hard-abort the process if called off it.

use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use std::thread;
use std::time::Duration;

#[cfg(target_os = "macos")]
const MODIFIER: Key = Key::Meta;
#[cfg(not(target_os = "macos"))]
const MODIFIER: Key = Key::Control;

pub fn paste_at_cursor(text: &str) -> anyhow::Result<()> {
    let mut clipboard = arboard::Clipboard::new()?;
    let previous = clipboard.get_text().ok();

    clipboard.set_text(text.to_string())?;
    // Give the OS a moment to register the clipboard change before we simulate paste.
    thread::sleep(Duration::from_millis(50));

    let mut enigo = Enigo::new(&Settings::default())?;
    press_paste(&mut enigo)?;

    restore_clipboard_later(previous);
    Ok(())
}

/// If the user has text selected in whatever app is focused, returns it — this is
/// how "select text, then dictate a command" is detected, with no language-based
/// guessing: write a unique sentinel value, simulate copy, and see whether the
/// clipboard now holds something else. Diffing against a sentinel (rather than
/// whatever was previously on the clipboard) avoids a real race: if the user selects
/// and re-copies the *same* text this app itself pasted moments ago — very likely,
/// since "dictate, then select what you just said to edit it" is the whole point —
/// a before/after diff against the old clipboard would see no change and wrongly
/// conclude nothing's selected.
pub fn capture_current_selection() -> anyhow::Result<Option<String>> {
    let mut clipboard = arboard::Clipboard::new()?;
    let previous = clipboard.get_text().ok();

    // No NUL bytes here — some clipboard/pasteboard string paths truncate at NUL.
    let sentinel = format!(
        "dictate-selection-probe-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    );
    clipboard.set_text(sentinel.clone())?;
    thread::sleep(Duration::from_millis(30));

    let mut enigo = Enigo::new(&Settings::default())?;
    enigo.key(MODIFIER, Direction::Press)?;
    enigo.key(Key::Unicode('c'), Direction::Click)?;
    enigo.key(MODIFIER, Direction::Release)?;
    thread::sleep(Duration::from_millis(80));

    let after = clipboard.get_text().ok();
    let selection = match &after {
        Some(text) if !text.is_empty() && text != &sentinel => Some(text.clone()),
        _ => None,
    };

    // We only wanted to read the selection, not permanently steal the clipboard —
    // put back whatever was there before.
    restore_clipboard_later(previous);
    Ok(selection)
}

fn press_paste(enigo: &mut Enigo) -> anyhow::Result<()> {
    enigo.key(MODIFIER, Direction::Press)?;
    enigo.key(Key::Unicode('v'), Direction::Click)?;
    enigo.key(MODIFIER, Direction::Release)?;
    Ok(())
}

/// Restores the user's previous clipboard contents shortly after, off this thread so
/// we don't delay returning control to the caller.
fn restore_clipboard_later(previous: Option<String>) {
    if let Some(previous) = previous {
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(800));
            if let Ok(mut clipboard) = arboard::Clipboard::new() {
                let _ = clipboard.set_text(previous);
            }
        });
    }
}
