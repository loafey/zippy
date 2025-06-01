use std::time::Instant;

fn fib(num: usize) -> usize {
    match num {
        0 => 0,
        1 => 1,
        _ => {
            if num <= 35 {
                fib(num - 1) + fib(num - 2)
            } else {
                let a = zippy::spawn(move || fib(num - 1));
                let b = zippy::spawn(move || fib(num - 2));
                a.wait() + b.wait()
            }
        }
    }
}

fn main() {
    let mut total = 0.0;
    let count = 200;
    for i in 0..count {
        println!("Test: {}/{count}", i + 1);
        let now = Instant::now();

        // let mut jobs = Vec::new();
        // for _ in 0..100000 {
        //     jobs.push(zippy::spawn(|| ()));
        // }
        // jobs.into_iter().for_each(|a| a.wait());

        zippy::spawn(|| fib(40)).wait();

        total += now.elapsed().as_secs_f64();
    }
    println!("took {:?}s on avg", total / count as f64);
    println!("{:?}", zippy::get_stats());
}
