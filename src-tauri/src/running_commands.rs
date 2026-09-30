use crate::tools::execute_command::kill_process_tree;
use shared_child::SharedChild;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

struct TrackedCommand {
    child: Arc<SharedChild>,
    cancelled: Arc<AtomicBool>,
}

/// Tracks terminal commands currently running via start_command_tool,
/// keyed by an opaque job id, so a separate cancel_command_tool call
/// (a different Tauri invocation entirely) can kill one by id. Only
/// commands started through start_command_tool — the cancellable
/// path used by the manual "Execute Terminal Command" panel — are
/// tracked here. The synchronous execute_command_tool used by
/// dispatch.rs and the agent loop is completely untouched and keeps
/// relying on its existing built-in timeout, same as before this
/// feature existed.
#[derive(Default)]
pub struct RunningCommands {
    next_id: AtomicU64,
    children: Mutex<HashMap<u64, TrackedCommand>>,
}

impl RunningCommands {
    pub fn register(&self, child: Arc<SharedChild>) -> (u64, Arc<AtomicBool>) {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let cancelled = Arc::new(AtomicBool::new(false));
        self.children.lock().unwrap().insert(
            id,
            TrackedCommand { child, cancelled: Arc::clone(&cancelled) },
        );
        (id, cancelled)
    }

    pub fn remove(&self, job_id: u64) {
        self.children.lock().unwrap().remove(&job_id);
    }

    pub fn cancel(&self, job_id: u64) -> Result<(), String> {
        let children = self.children.lock().unwrap();
        match children.get(&job_id) {
            Some(tracked) => {
                tracked.cancelled.store(true, Ordering::SeqCst);
                // See kill_process_tree's doc comment: a plain kill()
                // on the SharedChild only terminates cmd.exe, not
                // whatever program it spawned — which is exactly
                // what left ping.exe running and the stdout reader
                // blocked forever.
                kill_process_tree(tracked.child.id());
                Ok(())
            }
            None => Err(format!(
                "No running command found with id {} (it may have already finished)",
                job_id
            )),
        }
    }
}