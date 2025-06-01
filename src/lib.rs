#![warn(missing_docs, clippy::unwrap_used)]
#![doc = include_str!("../readme.md")]

use std::{
    collections::{BTreeMap, VecDeque},
    num::NonZero,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, Sender, channel},
    },
};

mod tests;

/// A job handle.
/// Works similarly to `std::thread::JoinHandle`.
/// A handle does not have to be consumed, and a task can simply
/// be forgotten if need be, as it will still be executed.
pub struct Job<T>(Receiver<T>);
impl<T> Job<T> {
    /// Blockingly wait for a job to finish.
    ///
    /// # Safety
    ///
    /// If the job thread panicked, so will the waiting thread.
    /// If panicking is a possibility [`Job::try_wait`] or [`Job::check`]
    /// should be used.
    ///
    /// ```
    /// # let expensive_operation1 = || 1;
    /// # let expensive_operation2 = || 2;
    /// let job1 = zippy::spawn(expensive_operation1);
    /// let job2 = zippy::spawn(expensive_operation2);
    /// let result = job1.wait() + job2.wait();
    /// ```
    pub fn wait(self) -> T {
        let recv = self.0;
        match recv.recv() {
            Ok(o) => o,
            Err(e) => panic!("a task paniced: {e}"),
        }
    }
    /// Blockingly wait for a job to finish.
    /// If the job has panicked, return [None].
    ///
    /// ```
    /// let job1: zippy::Job<usize> = zippy::spawn(|| 1);
    /// let job2: zippy::Job<usize> = zippy::spawn(|| panic!());
    /// assert_eq!(job1.try_wait(), Some(1));
    /// assert_eq!(job2.try_wait(), None);
    /// ```
    pub fn try_wait(self) -> Option<T> {
        let recv = self.0;
        recv.recv().ok()
    }

    /// Non-blockingly check if a job has finished.
    /// If not, return the Job handle again otherwise return the result.
    /// ```
    /// # use std::thread::sleep;
    /// # use std::time::Duration;
    ///
    /// let mut job = zippy::spawn(|| sleep(Duration::from_secs(1)));
    /// let result = loop {
    ///     match job.check() {
    ///         Ok(o) => break o,
    ///         Err(t) => {
    ///             job = t;
    ///             println!("not finished!");
    ///         },
    ///     }
    /// };
    /// println!("finished!");
    /// ```
    pub fn check(self) -> Result<T, Self> {
        match self.0.try_recv() {
            Ok(res) => Ok(res),
            Err(_) => Err(self),
        }
    }
}

struct Work {
    func: Box<dyn FnOnce()>,
}
unsafe impl Send for Work {}
unsafe impl Sync for Work {}

type Recursive = bool;
type Panic = bool;
enum WorkerMessage {
    Work(Work, Recursive),
    Done(usize, Panic),
    GetStats(Sender<PoolStats>),
}

/// A struct containing statistics about the workpool.
#[derive(Debug, Clone, Copy)]
pub struct PoolStats {
    /// How many workers are currently free.
    pub available_workers: usize,
    /// How many workers are currently working.
    pub taken_workers: usize,
    /// How many jobs exist in the backlog.
    pub backlog: usize,
    /// How many tasks have been sent to the pool.
    pub total_work_count: usize,
    /// How many rescue threads have been spawned for recursive jobs.
    pub total_rescue_threads: usize,
    /// How many worker threads have panicked. Does not account for rescue threads.
    pub total_panics: usize,
}

thread_local! {
    static ALREADY_WORKING: AtomicBool = AtomicBool::default();
}
fn send_to_manager(message: WorkerMessage) {
    let sender = HEAD_SENDER
        .get()
        .expect("somehow a manager thread has not been created");
    sender
        .send(message)
        .expect("the manager thread has crashed");
}

fn spawn_worker(i: usize) -> Sender<Work> {
    let (sender, recv) = channel::<Work>();
    std::thread::spawn(move || {
        let res = std::thread::Builder::new()
            .name(format!("Pool Worker {i}"))
            .spawn(move || {
                while let Ok(work) = recv.recv() {
                    ALREADY_WORKING.with(|a| a.store(true, Ordering::Relaxed));
                    (work.func)();
                    ALREADY_WORKING.with(|a| a.store(false, Ordering::Relaxed));
                    send_to_manager(WorkerMessage::Done(i, false));
                }
            })
            .expect("creating a worker thread failed")
            .join();

        if res.is_err() {
            send_to_manager(WorkerMessage::Done(i, true));
        }
    });
    sender
}

static HEAD_SENDER: OnceLock<Arc<Sender<WorkerMessage>>> = OnceLock::new();
fn manager_thread() {
    if HEAD_SENDER.get().is_none() {
        let thread_count = std::thread::available_parallelism()
            .unwrap_or_else(|_| NonZero::new(4).expect("4 == 0"))
            .get();
        let (s, r) = channel::<WorkerMessage>();
        HEAD_SENDER.get_or_init(|| Arc::new(s));
        std::thread::spawn(move || {
            let mut workers = BTreeMap::new();
            let mut taken = BTreeMap::new();
            // for i in 0..2 {
            for i in 0..thread_count {
                workers.insert(i, spawn_worker(i));
            }

            let mut backlog = VecDeque::new();
            let mut work_count: usize = 0;
            let mut rescue_threads: usize = 0;
            let mut panics: usize = 0;
            while let Ok(work) = r.recv() {
                match work {
                    WorkerMessage::Work(work, rec) => {
                        work_count = work_count.wrapping_add(1);
                        if let Some((index, worker)) = workers.pop_first() {
                            worker
                                .send(work)
                                .expect("a worker thread silently died while waiting for work");
                            taken.insert(index, worker);
                        } else if rec {
                            rescue_threads = rescue_threads.wrapping_add(1);
                            std::thread::Builder::new()
                                .name(format!("Rescue Thread {rescue_threads}"))
                                .spawn(move || {
                                    let work = work;
                                    ALREADY_WORKING.with(|a| a.store(true, Ordering::Relaxed));
                                    (work.func)();
                                })
                                .expect("creating a rescue thread failed");
                        } else {
                            backlog.push_back(work);
                        }
                    }
                    WorkerMessage::Done(i, panic) => {
                        let worker = if panic {
                            taken.remove(&i);
                            panics = panics.wrapping_add(1);
                            spawn_worker(i)
                        } else {
                            taken
                                .remove(&i)
                                .expect("a working thread silently got untracked")
                        };
                        workers.insert(i, worker);

                        if let Some(work) = backlog.pop_front() {
                            let worker = workers
                                .remove(&i)
                                .expect("an available thread silently got untracked");
                            worker
                                .send(work)
                                .expect("an available thread silently died");
                            taken.insert(i, worker);
                            if backlog.is_empty() {
                                backlog = VecDeque::new();
                            }
                        }
                    }
                    WorkerMessage::GetStats(sender) => sender
                        .send(PoolStats {
                            available_workers: workers.len(),
                            taken_workers: taken.len(),
                            backlog: backlog.len(),
                            total_work_count: work_count,
                            total_rescue_threads: rescue_threads,
                            total_panics: panics,
                        })
                        .expect("failed sending back statistics"),
                }
            }
        });
    }
}

/// Return some statistics about the workpool.
pub fn get_stats() -> PoolStats {
    manager_thread();
    let (res_send, res_recv) = channel();
    send_to_manager(WorkerMessage::GetStats(res_send));
    res_recv.recv().expect("failed getting statistics")
}

/// Send a job to the workpool.
/// Keep in mind that no promise is made about the execution order of jobs,
/// so if your jobs rely upon an order, you have to implement that yourself.
///
/// If a job is recursive (i.e a job that spawns more jobs),
/// and no other workers are available, the job will be executed on the current thread.
/// If this is not a wanted feature, use [`spawn_thread`] instead.
/// Rescue threads can still be spawned, but they
/// will be rarer when compared to [`spawn_thread`].
///
/// # Safety
/// If a job panics, and it is executing on the current thread, the current thread will
/// panic. Take care to write panic free code or use `spawn_thread` instead.
/// If you are writing a recursive function with this and it can panic,
/// call the outmost function using [`spawn_thread`].
pub fn spawn<T: 'static, F: FnOnce() -> T + 'static>(f: F) -> Job<T> {
    manager_thread();

    let (res_send, res_recv) = channel();
    let work = Work {
        func: Box::new(move || drop(res_send.send(f()))),
    };
    if get_stats().available_workers == 0 {
        (work.func)()
    } else {
        send_to_manager(WorkerMessage::Work(
            work,
            ALREADY_WORKING.with(|a| a.load(Ordering::Relaxed)),
        ));
    }
    Job(res_recv)
}

/// Send a job to the workpool.
/// Keep in mind that no promise is made about the execution order of jobs,
/// so if your jobs rely upon an order, you have to implement that yourself.
///
/// If a job is recursive (i.e a job that spawns more jobs),
/// these new jobs get flagged as recursive.
/// If there are no workers available when a recursive job is submitted
/// a new thread (refered to as a rescue thread)
/// will be spawned to handle this job, instead of being put
/// in the backlog.
/// This is done to avoid deadlocks where all workers are waiting on some other
/// worker to clean up the backlog.
pub fn spawn_thread<T: 'static, F: FnOnce() -> T + 'static>(f: F) -> Job<T> {
    manager_thread();

    let (res_send, res_recv) = channel();
    let work = Work {
        func: Box::new(move || drop(res_send.send(f()))),
    };

    send_to_manager(WorkerMessage::Work(
        work,
        ALREADY_WORKING.with(|a| a.load(Ordering::Relaxed)),
    ));
    Job(res_recv)
}
