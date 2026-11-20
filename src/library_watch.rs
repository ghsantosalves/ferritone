//! Keeps the library in step with its folders: a scan at startup, then a rescan shortly after
//! the file system reports a change.

use super::*;

/// Changes usually come in bursts (copying an album), so wait for them to settle.
const RESCAN_DELAY: Duration = Duration::from_secs(3);
/// Stays well below the default inotify watch limit, which other apps share.
const MAX_WATCHED_FOLDERS: usize = 4096;

impl Player {
    /// Scans every library folder in the background and applies what changed.
    pub(crate) fn rescan_library(self: &Rc<Self>) {
        if self.library_scanning.replace(true) {
            // A scan is running; run once more when it finishes, to catch this change too.
            self.library_rescan_again.set(true);
            return;
        }
        let roots: Vec<PathBuf> = self.state.borrow().library_roots.iter().cloned().collect();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let scans = gio::spawn_blocking(move || {
                roots
                    .iter()
                    .filter_map(|root| library::scan_root(root))
                    .collect::<Vec<_>>()
            })
            .await
            .unwrap_or_default();
            let Some(player) = weak.upgrade() else { return };
            let (added, removed) = player.state.borrow_mut().sync_roots(&scans);
            player.watch_folders(scans.iter().flat_map(|scan| scan.folders.iter()));
            if added + removed > 0 {
                player.refresh_library_visibility();
            }
            player.library_scanning.set(false);
            if player.library_rescan_again.replace(false) {
                player.rescan_library();
            }
        });
    }

    fn watch_folders<'a>(self: &Rc<Self>, folders: impl Iterator<Item = &'a PathBuf>) {
        let mut monitors = Vec::new();
        for folder in folders.take(MAX_WATCHED_FOLDERS) {
            let Ok(monitor) = gio::File::for_path(folder)
                .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
            else {
                continue;
            };
            let weak = Rc::downgrade(self);
            monitor.connect_changed(move |_, _, _, event| {
                use gio::FileMonitorEvent as Event;
                if !matches!(
                    event,
                    Event::Created
                        | Event::Deleted
                        | Event::MovedIn
                        | Event::MovedOut
                        | Event::Renamed
                ) {
                    return;
                }
                if let Some(player) = weak.upgrade() {
                    player.schedule_rescan();
                }
            });
            monitors.push(monitor);
        }
        // Dropping the old monitors stops watching folders that no longer exist.
        self.library_monitors.replace(monitors);
    }

    fn schedule_rescan(self: &Rc<Self>) {
        if let Some(pending) = self.library_rescan_timer.take() {
            pending.remove();
        }
        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_local_once(RESCAN_DELAY, move || {
            if let Some(player) = weak.upgrade() {
                player.library_rescan_timer.take();
                player.rescan_library();
            }
        });
        self.library_rescan_timer.replace(Some(source));
    }
}
