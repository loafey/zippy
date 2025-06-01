#![warn(missing_docs)]

use std::{
    collections::BTreeMap,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, Sender, channel},
    },
};

mod tests;

pub struct Job<T>(Receiver<T>);
impl<T> Job<T> {
    pub fn wait(self) -> T {
        let recv = self.0;
        recv.recv().unwrap()
    }
    pub fn try_wait(self) -> Option<T> {
        let recv = self.0;
        recv.recv().ok()
    }
    pub fn wait_non_blocking(self) -> Result<T, Self> {
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

#[derive(Debug, Clone, Copy)]
pub struct PoolStats {
    pub available_workers: usize,
    pub taken_workers: usize,
    pub backlog: usize,
    pub total_work_count: usize,
    pub total_rescue_threads: usize,
    pub total_panics: usize,
}

thread_local! {
    static ALREADY_WORKING: AtomicBool = AtomicBool::default();
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
                    let sender = HEAD_SENDER.get().unwrap();
                    sender.send(WorkerMessage::Done(i, false)).unwrap();
                }
            })
            .unwrap()
            .join();
        if res.is_err() {
            let sender = HEAD_SENDER.get().unwrap();
            sender.send(WorkerMessage::Done(i, true)).unwrap();
        }
    });
    sender
}

static HEAD_SENDER: OnceLock<Arc<Sender<WorkerMessage>>> = OnceLock::new();
fn manager_thread() {
    let thread_count = std::thread::available_parallelism().unwrap().get();

    if HEAD_SENDER.get().is_none() {
        let (s, r) = channel::<WorkerMessage>();
        HEAD_SENDER.get_or_init(|| Arc::new(s));
        std::thread::spawn(move || {
            let mut workers = BTreeMap::new();
            let mut taken = BTreeMap::new();
            // for i in 0..2 {
            for i in 0..thread_count {
                workers.insert(i, spawn_worker(i));
            }

            let mut backlog = Vec::new();
            let mut work_count: usize = 0;
            let mut rescue_threads: usize = 0;
            let mut panics: usize = 0;
            while let Ok(work) = r.recv() {
                match work {
                    WorkerMessage::Work(work, rec) => {
                        work_count = work_count.wrapping_add(1);
                        if workers.is_empty() {
                            if rec {
                                rescue_threads = rescue_threads.wrapping_add(1);
                                std::thread::Builder::new()
                                    .name(format!("Rescue Thread {rescue_threads}"))
                                    .spawn(move || {
                                        let work = work;
                                        ALREADY_WORKING.with(|a| a.store(true, Ordering::Relaxed));
                                        (work.func)();
                                    })
                                    .unwrap();
                            } else {
                                backlog.push(work);
                            }
                        } else {
                            let (index, worker) = workers.pop_first().unwrap();
                            worker.send(work).unwrap();
                            taken.insert(index, worker);
                        }
                    }
                    WorkerMessage::Done(i, panic) => {
                        let worker = if panic {
                            taken.remove(&i);
                            panics = panics.wrapping_add(1);
                            spawn_worker(i)
                        } else {
                            taken.remove(&i).unwrap()
                        };
                        workers.insert(i, worker);

                        if let Some(work) = backlog.pop() {
                            let worker = workers.remove(&i).unwrap();
                            worker.send(work).unwrap();
                            taken.insert(i, worker);
                            if backlog.is_empty() {
                                backlog = Vec::new();
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
                        .unwrap(),
                }
            }
        });
    }
}

pub fn get_stats() -> PoolStats {
    manager_thread();
    let sender = HEAD_SENDER.get().unwrap();
    let (res_send, res_recv) = channel();
    sender.send(WorkerMessage::GetStats(res_send)).unwrap();
    res_recv.recv().unwrap()
}

pub fn send_work<T: 'static, F: FnOnce() -> T + 'static>(f: F) -> Job<T> {
    manager_thread();

    let sender = HEAD_SENDER.get().unwrap();

    let (res_send, res_recv) = channel();
    sender
        .send(WorkerMessage::Work(
            Work {
                func: Box::new(move || {
                    res_send.send(f()).unwrap();
                }),
            },
            ALREADY_WORKING.with(|a| a.load(Ordering::Relaxed)),
        ))
        .unwrap();
    Job(res_recv)
}
