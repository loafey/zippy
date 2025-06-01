use std::{
    collections::BTreeMap,
    sync::{
        Arc, LazyLock, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, Sender, channel},
    },
    thread::sleep,
    time::Duration,
};

struct Job<T>(Receiver<T>);
impl<T> Job<T> {
    pub fn wait(self) -> T {
        self.0.recv().unwrap()
    }
    pub fn try_wait(self) -> Result<T, Self> {
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
enum WorkerMessage {
    Work(Work, Recursive),
    Done(usize),
    GetStats(Sender<PoolStats>),
}

static THREAD_COUNT: LazyLock<usize> =
    LazyLock::new(|| std::thread::available_parallelism().unwrap().get());

#[derive(Debug, Clone, Copy)]
pub struct PoolStats {
    pub available_workers: usize,
    pub taken_workers: usize,
    pub backlog: usize,
    pub total_work_count: usize,
    pub total_rescue_threads: usize,
}

thread_local! {
    static ALREADY_WORKING: AtomicBool = AtomicBool::default();
}
static HEAD_SENDER: OnceLock<Arc<Sender<WorkerMessage>>> = OnceLock::new();
fn manager_thread() {
    if HEAD_SENDER.get().is_none() {
        let (s, r) = channel::<WorkerMessage>();
        HEAD_SENDER.set(Arc::new(s)).unwrap();
        std::thread::spawn(move || {
            let mut workers = BTreeMap::new();
            let mut taken = BTreeMap::new();
            for i in 0..*THREAD_COUNT {
                let (sender, recv) = channel::<Work>();
                std::thread::spawn(move || {
                    while let Ok(work) = recv.recv() {
                        ALREADY_WORKING.with(|a| a.store(true, Ordering::Relaxed));
                        (work.func)();
                        ALREADY_WORKING.with(|a| a.store(false, Ordering::Relaxed));
                        let sender = HEAD_SENDER.get().unwrap();
                        sender.send(WorkerMessage::Done(i)).unwrap();
                    }
                });
                workers.insert(i, sender);
            }

            let mut backlog = Vec::new();
            let mut work_count: usize = 0;
            let mut rescue_threads: usize = 0;
            macro_rules! print_info {
                () => {
                    // println!("Got work: {work_count}:{}:{rescue_threads}", backlog.len());
                };
            }
            while let Ok(work) = r.recv() {
                match work {
                    WorkerMessage::Work(work, rec) => {
                        work_count = work_count.wrapping_add(1);
                        print_info!();
                        if workers.is_empty() {
                            if rec {
                                rescue_threads = rescue_threads.wrapping_add(1);
                                std::thread::spawn(move || {
                                    let work = work;
                                    ALREADY_WORKING.with(|a| a.store(true, Ordering::Relaxed));
                                    (work.func)();
                                });
                            } else {
                                backlog.push(work);
                            }
                        } else {
                            let (index, worker) = workers.pop_first().unwrap();
                            worker.send(work).unwrap();
                            taken.insert(index, worker);
                        }
                    }
                    WorkerMessage::Done(i) => {
                        if let Some(work) = backlog.pop() {
                            print_info!();
                            taken.get(&i).unwrap().send(work).unwrap();
                            if backlog.is_empty() {
                                backlog = Vec::new();
                            }
                        } else {
                            print_info!();
                            let worker = taken.remove(&i).unwrap();
                            workers.insert(i, worker);
                        }
                    }
                    WorkerMessage::GetStats(sender) => sender
                        .send(PoolStats {
                            available_workers: workers.len(),
                            taken_workers: taken.len(),
                            backlog: backlog.len(),
                            total_work_count: work_count,
                            total_rescue_threads: rescue_threads,
                        })
                        .unwrap(),
                }
            }
        });
    }
}
fn get_stats() -> PoolStats {
    manager_thread();
    let sender = HEAD_SENDER.get().unwrap();
    let (res_send, res_recv) = channel();
    sender.send(WorkerMessage::GetStats(res_send)).unwrap();
    res_recv.recv().unwrap()
}
fn send_work<T: 'static, F: FnOnce() -> T + 'static>(f: F) -> Job<T> {
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

fn fib(num: usize) -> usize {
    match num {
        0 => 0,
        1 => 1,
        _ => {
            let a = send_work(move || fib(num - 1)).wait();
            let b = send_work(move || fib(num - 2)).wait();
            a + b
        }
    }
}

fn main() {
    // let mut jobs = Vec::new();
    // for i in 0..100 {
    // jobs.push(send_work(move || id(i)));
    // }
    // println!("{}", jobs.into_iter().map(|a| a.wait()).sum::<usize>());
    let start_val = 24;
    let mut task = send_work(move || fib(start_val));
    loop {
        match task.try_wait() {
            Ok(res) => {
                println!("Fibbo done: {res}!");
                break;
            }
            Err(t) => {
                task = t;
                let res = get_stats();
                println!("Waiting: {res:?}...",);
                sleep(Duration::from_millis(8));
            }
        }
    }
    println!("Final stats: {:?}", get_stats())
}
