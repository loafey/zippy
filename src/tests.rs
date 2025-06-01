#![allow(clippy::unwrap_used)]

use std::sync::{LazyLock, Mutex};

#[allow(unused)]
static TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(Mutex::default);

#[test]
fn linear() {
    let op = TEST_LOCK.lock().unwrap();

    let before_stats = crate::get_stats();
    let mut jobs = Vec::new();
    let count = 100000;
    for _ in 0..count {
        jobs.push(crate::spawn_rec(move || 1));
    }
    let res = jobs.into_iter().map(|a| a.wait()).sum::<usize>();
    let after_stats = crate::get_stats();

    assert_eq!(res, count);
    assert_eq!(
        before_stats.total_work_count + count,
        after_stats.total_work_count
    );
    assert_eq!(
        before_stats.available_workers,
        after_stats.available_workers
    );
    assert_eq!(before_stats.taken_workers, after_stats.taken_workers);
    assert_eq!(before_stats.total_panics, after_stats.total_panics);
    assert_eq!(
        before_stats.total_rescue_threads,
        after_stats.total_rescue_threads
    );

    drop(op)
}

#[test]
fn recursive() {
    let op = TEST_LOCK.lock().unwrap();

    fn fib(num: usize) -> usize {
        match num {
            0 | 1 => num,
            _ => {
                let a = crate::spawn_rec(move || fib(num - 1));
                let b = crate::spawn_rec(move || fib(num - 2));
                a.wait() + b.wait()
            }
        }
    }

    let before_stats = crate::get_stats();
    let res = crate::spawn_rec(|| fib(22)).wait();
    let after_stats = crate::get_stats();

    assert_eq!(res, 17711);
    assert!(before_stats.total_work_count < after_stats.total_work_count);
    assert_eq!(
        before_stats.available_workers,
        after_stats.available_workers
    );
    assert_eq!(before_stats.taken_workers, after_stats.taken_workers);
    assert_eq!(before_stats.total_panics, after_stats.total_panics);
    assert!(before_stats.total_rescue_threads < after_stats.total_rescue_threads);

    drop(op)
}

#[test]
fn crash() {
    let op = TEST_LOCK.lock().unwrap();

    let mut jobs = Vec::new();

    fn panic_job() {
        let rand = rand::random::<u8>() % 2;
        if rand == 0 {
            panic!("OUCH!")
        }
    }
    let before_stats = crate::get_stats();

    let count = 100000;
    for _ in 0..count {
        jobs.push(crate::spawn_rec(panic_job));
    }

    let mut sum = 0;
    while let Some(w) = jobs.pop() {
        if w.try_wait().is_some() {
            sum += 1;
        } else {
            jobs.push(crate::spawn_rec(panic_job));
        }
    }
    let after_stats = crate::get_stats();

    assert_eq!(sum, count);
    assert!(before_stats.total_work_count < after_stats.total_work_count);
    assert_eq!(
        before_stats.available_workers,
        after_stats.available_workers
    );
    assert_eq!(before_stats.taken_workers, after_stats.taken_workers);
    assert!(before_stats.total_panics <= after_stats.total_panics);
    assert_eq!(
        before_stats.total_rescue_threads,
        after_stats.total_rescue_threads
    );

    drop(op)
}
