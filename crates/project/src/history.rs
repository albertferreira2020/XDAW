//! Undo/redo through reversible edits.
//!
//! Each edit stores the before/after value of the smallest sub-tree it
//! touches; applying the inverse swaps them. Continuous gestures (fader drags)
//! are recorded once at gesture end.

use crate::model::{Project, Scene, Slot, Track, TrackRef, TransportSettings};

#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    /// Replace one track (clips, devices, mixer, arrangement, automation).
    Track { at: TrackRef, before: Box<Track>, after: Box<Track> },
    InsertTrack { index: usize, track: Box<Track> },
    RemoveTrack { index: usize, track: Box<Track> },
    Transport { before: TransportSettings, after: TransportSettings },
    /// Scene list plus every track's slots (scene insert/delete/reorder).
    Scenes { before: Box<(Vec<Scene>, Vec<Vec<Slot>>)>, after: Box<(Vec<Scene>, Vec<Vec<Slot>>)> },
    /// Fallback for edits spanning the whole project (add return, load preset sets).
    Project { before: Box<Project>, after: Box<Project> },
}

impl Edit {
    fn apply(&self, p: &mut Project) {
        match self {
            Edit::Track { at, after, .. } => *p.track_mut(*at) = (**after).clone(),
            Edit::InsertTrack { index, track } => p.tracks.insert(*index, (**track).clone()),
            Edit::RemoveTrack { index, .. } => {
                p.tracks.remove(*index);
            }
            Edit::Transport { after, .. } => p.transport = after.clone(),
            Edit::Scenes { after, .. } => set_scenes(p, after),
            Edit::Project { after, .. } => {
                let next_id = p.next_id.max(after.next_id);
                *p = (**after).clone();
                p.next_id = next_id;
            }
        }
    }

    fn inverse(&self) -> Edit {
        match self.clone() {
            Edit::Track { at, before, after } => Edit::Track { at, before: after, after: before },
            Edit::InsertTrack { index, track } => Edit::RemoveTrack { index, track },
            Edit::RemoveTrack { index, track } => Edit::InsertTrack { index, track },
            Edit::Transport { before, after } => Edit::Transport { before: after, after: before },
            Edit::Scenes { before, after } => Edit::Scenes { before: after, after: before },
            Edit::Project { before, after } => Edit::Project { before: after, after: before },
        }
    }
}

fn set_scenes(p: &mut Project, v: &(Vec<Scene>, Vec<Vec<Slot>>)) {
    p.scenes = v.0.clone();
    for (t, slots) in p.tracks.iter_mut().zip(&v.1) {
        t.slots = slots.clone();
    }
}

pub fn scenes_snapshot(p: &Project) -> Box<(Vec<Scene>, Vec<Vec<Slot>>)> {
    Box::new((p.scenes.clone(), p.tracks.iter().map(|t| t.slots.clone()).collect()))
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub label: String,
    pub edit: Edit,
}

#[derive(Default, Debug)]
pub struct History {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    /// Increments on every change; the UI compares it with the saved revision.
    pub revision: u64,
}

pub const MAX_HISTORY: usize = 500;

impl History {
    /// Applies an edit and records it.
    pub fn apply(&mut self, p: &mut Project, label: impl Into<String>, edit: Edit) {
        edit.apply(p);
        self.record(label, edit);
    }

    /// Records an edit whose effect is already present in the project.
    pub fn record(&mut self, label: impl Into<String>, edit: Edit) {
        self.undo.push(Entry { label: label.into(), edit });
        if self.undo.len() > MAX_HISTORY {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.revision += 1;
    }

    /// Convenience: mutate one track and record the change.
    pub fn edit_track(&mut self, p: &mut Project, at: TrackRef, label: &str, f: impl FnOnce(&mut Track)) {
        let before = Box::new(p.track(at).clone());
        f(p.track_mut(at));
        let after = Box::new(p.track(at).clone());
        if before != after {
            self.record(label, Edit::Track { at, before, after });
        }
    }

    /// Mutate the whole project and record a project-level edit.
    pub fn edit_project(&mut self, p: &mut Project, label: &str, f: impl FnOnce(&mut Project)) {
        let before = Box::new(p.clone());
        f(p);
        if *before != *p {
            let after = Box::new(p.clone());
            self.record(label, Edit::Project { before, after });
        }
    }

    pub fn undo(&mut self, p: &mut Project) -> Option<String> {
        let e = self.undo.pop()?;
        e.edit.inverse().apply(p);
        self.revision += 1;
        let label = e.label.clone();
        self.redo.push(e);
        Some(label)
    }

    pub fn redo(&mut self, p: &mut Project) -> Option<String> {
        let e = self.redo.pop()?;
        e.edit.apply(p);
        self.revision += 1;
        let label = e.label.clone();
        self.undo.push(e);
        Some(label)
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|e| e.label.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|e| e.label.as_str())
    }
}
