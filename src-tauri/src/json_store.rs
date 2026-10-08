//! The JSON files History, Voice Notes and Transcripts keep their lists in.

use std::{fs, path::Path};

use serde::{Serialize, de::DeserializeOwned};

/// The value in `file`, or the default when there is none. An unreadable
/// file is set aside first, as settings are, so the next save does not
/// overwrite the only copy. `what` names it in the log.
pub fn load<T: DeserializeOwned + Default>(file: &Path, what: &str) -> T {
    let Ok(text) = fs::read_to_string(file) else {
        return T::default();
    };
    serde_json::from_str(&text).unwrap_or_else(|error| {
        let backup = file.with_extension("json.bak");
        match fs::copy(file, &backup) {
            Ok(_) => tracing::warn!(%error, backup = %backup.display(), "{what} unreadable; starting empty"),
            Err(copy) => tracing::error!(%error, %copy, "{what} unreadable and not backed up; starting empty"),
        }
        T::default()
    })
}

/// Writes `value` to `file` through a temporary file, so a crash midway
/// leaves the old one. Failures are logged.
pub fn save<T: Serialize + ?Sized>(file: &Path, value: &T, what: &str) {
    let result = file
        .parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| serde_json::to_string(value).map_err(std::io::Error::other))
        .and_then(|text| {
            let tmp = file.with_extension("json.tmp");
            fs::write(&tmp, text)?;
            fs::rename(&tmp, file)
        });
    if let Err(error) = result {
        tracing::error!(%error, "cannot save {what}");
    }
}
