//! Where Kite keeps its data, and the one-time move from the folder used
//! before the app was named.

use std::path::PathBuf;

/// `~/Library/Application Support/Kite`: credentials, the TDLib session
/// and database, recent stickers.
pub fn data_dir() -> Option<PathBuf> {
    Some(home()?.join("Library/Application Support/Kite"))
}

/// Before the rename, data lived under "Telegram GPUI". Move it over once,
/// so the existing session keeps working. Leaves both alone if the new
/// folder already exists.
pub fn migrate_legacy() {
    let (Some(home), Some(new)) = (home(), data_dir()) else {
        return;
    };
    let old = home.join("Library/Application Support/Telegram GPUI");
    if old.is_dir()
        && !new.exists()
        && let Err(error) = std::fs::rename(&old, &new)
    {
        eprintln!(
            "Could not move {} to {}: {error}",
            old.display(),
            new.display()
        );
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}
