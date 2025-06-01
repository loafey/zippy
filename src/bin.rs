use std::time::Instant;

fn fib(num: usize) -> usize {
    match num {
        0 => 0,
        1 => 1,
        _ => {
            let a = zippy::spawn(move || fib(num - 1));
            let b = zippy::spawn(move || fib(num - 2));
            a.wait() + b.wait()
        }
    }
}

fn main() {
    let now = Instant::now();
    zippy::spawn(|| fib(20)).wait();
    println!("took {:?}", now.elapsed())
}
