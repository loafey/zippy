use std::{
    collections::BTreeMap,
    sync::{
        Arc, LazyLock, OnceLock,
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
}

struct Work {
    func: Box<dyn FnOnce()>,
}
unsafe impl Send for Work {}
unsafe impl Sync for Work {}

enum WorkerMessage {
    Work(Work),
    Done(usize),
}

static THREAD_COUNT: LazyLock<usize> =
    LazyLock::new(|| std::thread::available_parallelism().unwrap().get());

static HEAD_SENDER: OnceLock<Arc<Sender<WorkerMessage>>> = OnceLock::new();
fn send_work<T: 'static, F: FnOnce() -> T + 'static>(f: F) -> Job<T> {
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
                        (work.func)();
                        let sender = HEAD_SENDER.get().unwrap();
                        sender.send(WorkerMessage::Done(i)).unwrap();
                        println!("normal thread done");
                    }
                });
                workers.insert(i, sender);
            }

            let mut backlog = Vec::new();
            let mut work_count = 0;
            while let Ok(work) = r.recv() {
                match work {
                    WorkerMessage::Work(work) => {
                        work_count += 1;
                        println!("Got work: {work_count} {}", backlog.len());
                        if workers.is_empty() {
                            if backlog.len() > *THREAD_COUNT {
                                std::thread::spawn(move || {
                                    let work = work;
                                    (work.func)();
                                    println!("backlog thread done");
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
                            taken.get(&i).unwrap().send(work).unwrap();
                            if backlog.is_empty() {
                                backlog = Vec::new();
                            }
                        } else {
                            let worker = taken.remove(&i).unwrap();
                            workers.insert(i, worker);
                        }
                    }
                }
            }
        });
    }

    let sender = HEAD_SENDER.get().unwrap();
    let (res_send, res_recv) = channel();
    sender
        .send(WorkerMessage::Work(Work {
            func: Box::new(move || {
                res_send.send(f()).unwrap();
            }),
        }))
        .unwrap();
    Job(res_recv)
}

fn fib(num: usize) -> usize {
    match num {
        0 => 0,
        1 => 1,
        _ => {
            let a = send_work(move || fib(num - 1)).wait();
            let b = send_work(move || fib(num - 2));
            a + b.wait()
        }
    }
}

fn main() {
    // let a = send_work(|| {
    //     sleep(Duration::from_secs(1));
    //     1
    // });
    // let b = send_work(|| {
    //     sleep(Duration::from_secs(1));
    //     2
    // });
    // let c = send_work(move || a.wait() + b.wait());

    // println!("{}", c.wait());ä
    println!("{}", fib(10));
}
