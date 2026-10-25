//! File-level undo/redo history for a place file.
//!
//! Each place can have a sibling `<place>.bloxhistory/` directory holding
//! raw snapshots of the place file plus a `state.json` manifest with the
//! undo and redo stacks (snapshot file names, most recent last). CLI
//! processes are short-lived, so history lives on disk next to the place —
//! never inside the `.rbxl` itself. Snapshots are restored only after they
//! decode successfully, and restores are atomic.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::rbxio::write_atomic;
use crate::protocol::tree::{Error, Result};

const MAX_SNAPSHOTS: usize = 100;

#[derive(Debug, Default, Serialize, Deserialize)]
struct HistoryState {
    #[serde(default)]
    undo: Vec<String>,
    #[serde(default)]
    redo: Vec<String>,
    #[serde(default)]
    next_id: u64,
}

pub struct History {
    dir: PathBuf,
    state: HistoryState,
}

pub fn history_dir(place: &Path) -> PathBuf {
    let mut name = place
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "place".to_string());
    name.push_str(".bloxhistory");
    match place.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(name),
        _ => PathBuf::from(name),
    }
}

impl History {
    pub fn open(place: &Path) -> Result<Self> {
        let dir = history_dir(place);
        let state = match std::fs::read(dir.join("state.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| Error(format!("corrupt history manifest in {}: {e}", dir.display())))?,
            Err(_) => HistoryState::default(),
        };
        Ok(Self { dir, state })
    }

    fn save(&self) -> Result<()> {
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| Error(format!("cannot create {}: {e}", self.dir.display())))?;
        let bytes = serde_json::to_vec_pretty(&self.state)
            .map_err(|e| Error(format!("cannot serialize history manifest: {e}")))?;
        write_atomic(&self.dir.join("state.json"), &bytes)
    }

    fn store_snapshot(&mut self, place: &Path) -> Result<String> {
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| Error(format!("cannot create {}: {e}", self.dir.display())))?;
        self.state.next_id += 1;
        let ext = place
            .extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_else(|| "rbxl".to_string());
        let name = format!("snap-{:06}.{ext}", self.state.next_id);
        let bytes = std::fs::read(place)
            .map_err(|e| Error(format!("cannot read {} for history: {e}", place.display())))?;
        write_atomic(&self.dir.join(&name), &bytes)?;
        Ok(name)
    }

    fn prune(&mut self) {
        while self.state.undo.len() + self.state.redo.len() > MAX_SNAPSHOTS {
            let removed = if !self.state.undo.is_empty() {
                self.state.undo.remove(0)
            } else {
                self.state.redo.remove(0)
            };
            let _ = std::fs::remove_file(self.dir.join(removed));
        }
    }

    /// Records the place's current on-disk state as an undo point. Called by
    /// mutating commands after the engine accepted the change but before the
    /// new file is written. A fresh mutation clears the redo stack.
    pub fn record(&mut self, place: &Path) -> Result<()> {
        if !place.is_file() {
            return Ok(());
        }
        let name = self.store_snapshot(place)?;
        self.state.undo.push(name);
        for redo in std::mem::take(&mut self.state.redo) {
            let _ = std::fs::remove_file(self.dir.join(redo));
        }
        self.prune();
        self.save()
    }

    pub fn can_undo(&self) -> bool {
        !self.state.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.state.redo.is_empty()
    }
    pub fn undo_depth(&self) -> usize {
        self.state.undo.len()
    }
    pub fn redo_depth(&self) -> usize {
        self.state.redo.len()
    }

    /// Restores the most recent undo snapshot. The current file becomes a
    /// redo point. Returns the restored snapshot's file name.
    pub fn undo(&mut self, place: &Path) -> Result<String> {
        let Some(name) = self.state.undo.pop() else {
            return Err(Error(format!(
                "nothing to undo for '{}' (no history in {})",
                place.display(),
                self.dir.display()
            )));
        };
        // The snapshot must decode before we touch the place file.
        let snapshot_path = self.dir.join(&name);
        crate::rbxio::read_place(&snapshot_path).map_err(|e| {
            Error(format!(
                "history snapshot {} is corrupt, refusing to restore: {}",
                snapshot_path.display(),
                e.0
            ))
        })?;
        if place.is_file() {
            let current = self.store_snapshot(place)?;
            self.state.redo.push(current);
        }
        let bytes = std::fs::read(&snapshot_path)
            .map_err(|e| Error(format!("cannot read {}: {e}", snapshot_path.display())))?;
        write_atomic(place, &bytes)?;
        let _ = std::fs::remove_file(&snapshot_path);
        self.prune();
        self.save()?;
        Ok(name)
    }

    /// Re-applies the most recently undone snapshot.
    pub fn redo(&mut self, place: &Path) -> Result<String> {
        let Some(name) = self.state.redo.pop() else {
            return Err(Error(format!(
                "nothing to redo for '{}'",
                place.display()
            )));
        };
        let snapshot_path = self.dir.join(&name);
        crate::rbxio::read_place(&snapshot_path).map_err(|e| {
            Error(format!(
                "history snapshot {} is corrupt, refusing to restore: {}",
                snapshot_path.display(),
                e.0
            ))
        })?;
        if place.is_file() {
            let current = self.store_snapshot(place)?;
            self.state.undo.push(current);
        }
        let bytes = std::fs::read(&snapshot_path)
            .map_err(|e| Error(format!("cannot read {}: {e}", snapshot_path.display())))?;
        write_atomic(place, &bytes)?;
        let _ = std::fs::remove_file(&snapshot_path);
        self.prune();
        self.save()?;
        Ok(name)
    }
}
