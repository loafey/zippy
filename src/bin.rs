#![feature(random)]

use std::random::random;

fn fib(num: usize) -> usize {
    match num {
        0 => 0,
        1 => 1,
        _ => {
            let a = zippy::spawn(move || fib(num - 1)).wait();
            let b = zippy::spawn(move || fib(num - 2)).wait();
            a + b
        }
    }
}

fn main() {
    // println!("{}", jobs.into_iter().map(|a| a.wait()).sum::<usize>());
    zippy::spawn(|| fib(20)).wait();
    println!("After recursive test stats: {:#?}", zippy::get_stats());
    let mut jobs = Vec::new();
    for i in 0..100000 {
        jobs.push(zippy::spawn(move || i));
    }
    jobs.into_iter().map(|a| a.wait()).sum::<usize>();
    println!(
        "After a lot of linear work stats: {:#?}",
        zippy::get_stats()
    );

    let mut jobs = Vec::new();
    fn panic_job() {
        let rand = random::<usize>() % 2;
        if rand == 0 {
            panic!("OUCH!")
        }
    }
    for _ in 0..100000 {
        jobs.push(zippy::spawn(panic_job));
    }

    let mut sum = 0;
    while let Some(w) = jobs.pop() {
        if w.try_wait().is_some() {
            sum += 1;
        } else {
            jobs.push(zippy::spawn(panic_job));
        }
    }
    println!(
        "After crash test stats (sum: {sum}): {:#?}",
        zippy::get_stats()
    );
}
