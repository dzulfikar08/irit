//! Background loads the window starts, cancels and replaces: which one is current, and what to do
//! with a result when it arrives. The alignment already open is not touched by a load until a new
//! one has finished; a cancelled or replaced load's result is dropped when it comes back, which
//! frees its memory, and the open alignment stays.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use crate::parse::Progress;

/// A load in progress.
#[derive(Debug)]
pub struct Job {
    pub id: u64,
    pub path: PathBuf,
    pub progress: Arc<Progress>,
    pub started: Instant,
}

/// At most one load at a time.
#[derive(Debug, Default)]
pub struct Jobs {
    job: Option<Job>,
    next: u64,
}

/// What [`Jobs::finish`] decided about a result.
#[derive(Debug)]
pub enum Done<T> {
    /// The current load ended; show this (an alignment or the error).
    Current(PathBuf, anyhow::Result<T>),
    /// A load that was cancelled or replaced: its result has been dropped.
    Stale,
}

impl Jobs {
    /// Start a load of `path`, cancelling the one running. `total` is the file size for the
    /// progress bar.
    pub fn start(&mut self, path: PathBuf, total: u64) -> &Job {
        self.cancel();
        self.next += 1;
        self.job = Some(Job {
            id: self.next,
            path,
            progress: Arc::new(Progress::new(total)),
            started: Instant::now(),
        });
        self.job.as_ref().unwrap()
    }

    /// Cancel the running load: its reader stops at the next check and its result will be
    /// [`Done::Stale`]. Returns the path it was loading.
    pub fn cancel(&mut self) -> Option<PathBuf> {
        let j = self.job.take()?;
        j.progress.cancel();
        Some(j.path)
    }

    pub fn current(&self) -> Option<&Job> {
        self.job.as_ref()
    }

    pub fn is_current(&self, id: u64) -> bool {
        self.job.as_ref().is_some_and(|j| j.id == id)
    }

    /// The result of load `id`: the current load's result, or `Stale` (and the result dropped)
    /// for one that was cancelled or replaced.
    pub fn finish<T>(&mut self, id: u64, result: anyhow::Result<T>) -> Done<T> {
        if !self.is_current(id) {
            drop(result);
            return Done::Stale;
        }
        let j = self.job.take().unwrap();
        Done::Current(j.path, result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Counts drops, standing in for a loaded alignment.
    struct Big(Arc<AtomicUsize>);
    impl Drop for Big {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn cancel_drops_the_result_and_keeps_the_open_file() {
        let drops = Arc::new(AtomicUsize::new(0));
        let mut jobs = Jobs::default();
        // The file already open, held by the window.
        let open = Some(Big(drops.clone()));
        let id = jobs.start("big.fa".into(), 100).id;
        let p = jobs.current().unwrap().progress.clone();
        assert_eq!(jobs.cancel(), Some(PathBuf::from("big.fa")));
        assert!(p.is_cancelled() && jobs.current().is_none());
        // The background load returns later: dropped at once, the open file untouched.
        assert!(matches!(
            jobs.finish(id, Ok(Big(drops.clone()))),
            Done::Stale
        ));
        assert_eq!(drops.load(Ordering::Relaxed), 1);
        assert!(open.is_some());
        drop(open);
        assert_eq!(drops.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn a_new_load_replaces_the_running_one() {
        let mut jobs = Jobs::default();
        let a = jobs.start("a.fa".into(), 1).id;
        let pa = jobs.current().unwrap().progress.clone();
        let b = jobs.start("b.fa".into(), 1).id;
        assert!(pa.is_cancelled());
        assert!(matches!(jobs.finish::<u8>(a, Ok(1)), Done::Stale));
        match jobs.finish::<u8>(b, Ok(2)) {
            Done::Current(p, Ok(2)) => assert_eq!(p, PathBuf::from("b.fa")),
            other => panic!("{other:?}"),
        }
        // Finished: nothing current, a repeat is stale.
        assert!(jobs.current().is_none());
        assert!(matches!(jobs.finish::<u8>(b, Ok(3)), Done::Stale));
    }

    #[test]
    fn errors_of_the_current_load_are_reported() {
        let mut jobs = Jobs::default();
        let id = jobs.start("bad.fa".into(), 1).id;
        assert!(matches!(
            jobs.finish::<u8>(id, Err(anyhow::anyhow!("bad"))),
            Done::Current(_, Err(_))
        ));
    }
}
