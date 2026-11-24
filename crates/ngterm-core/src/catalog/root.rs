//! The catalog's durable id (upstream's `CatalogRoot.swift`): one per server state directory, so a remote window bound
//! to this server keeps seeing the same catalog across its restarts.

use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use uuid::Uuid;

use crate::pane_env::uuid_string;
use crate::persist::json::write_atomic;
use crate::persist::parse_uuid_string;

/// The file in the state directory that holds the id.
pub const FILE_NAME: &str = "catalog-id";

/// Why the id could not be read; the file is left alone either way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogIdError {
    Unreadable(String),
    Malformed(String),
}

impl fmt::Display for CatalogIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable(path) => write!(f, "catalog id cannot be read: {path}"),
            Self::Malformed(path) => write!(f, "catalog id is not a UUID: {path}"),
        }
    }
}

impl std::error::Error for CatalogIdError {}

/// Reads the catalog id from `directory`, writing a new one only when the file does not exist. A file that cannot be
/// read or holds anything else is an error and stays as it is: a new id would make every viewer bound to this server
/// see a different catalog.
///
/// # Errors
///
/// The file exists and cannot be read, holds no UUID, or a new one cannot be written.
pub fn load(directory: &Path) -> Result<Uuid, CatalogIdError> {
    let file = directory.join(FILE_NAME);
    let shown = || file.display().to_string();
    match fs::read_to_string(&file) {
        Ok(text) => parse_uuid_string(text.trim()).ok_or_else(|| CatalogIdError::Malformed(shown())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let id = Uuid::new_v4();
            fs::create_dir_all(directory)
                .and_then(|()| write_atomic(&file, format!("{}\n", uuid_string(id)).as_bytes()))
                .map_err(|_| CatalogIdError::Unreadable(shown()))?;
            Ok(id)
        }
        Err(_) => Err(CatalogIdError::Unreadable(shown())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Upstream's `theFirstLoadWritesAnIdThatLaterLoadsKeep`.
    #[test]
    fn the_first_load_writes_an_id_that_later_loads_keep() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let first = load(&state).unwrap();
        assert_eq!(load(&state), Ok(first));
        let written = fs::read_to_string(state.join(FILE_NAME)).unwrap();
        assert_eq!(written, format!("{}\n", uuid_string(first)));
    }

    /// Upstream's `aMalformedIdIsAnErrorAndLeftAlone`.
    #[test]
    fn a_malformed_id_is_an_error_and_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE_NAME);
        fs::write(&file, "not-a-uuid\n").unwrap();
        assert_eq!(
            load(dir.path()),
            Err(CatalogIdError::Malformed(file.display().to_string()))
        );
        assert_eq!(fs::read_to_string(&file).unwrap(), "not-a-uuid\n");
        assert_eq!(
            CatalogIdError::Malformed("/s/catalog-id".to_owned()).to_string(),
            "catalog id is not a UUID: /s/catalog-id"
        );
    }

    #[test]
    fn an_unreadable_id_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE_NAME);
        fs::create_dir(&file).unwrap();
        assert_eq!(
            load(dir.path()),
            Err(CatalogIdError::Unreadable(file.display().to_string()))
        );
        assert_eq!(
            CatalogIdError::Unreadable("/s/catalog-id".to_owned()).to_string(),
            "catalog id cannot be read: /s/catalog-id"
        );
    }
}
