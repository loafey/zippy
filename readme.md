# zippy

Dead-simple and naive zero-dependency workpool implementation.

```rs
fn main() {
    let job1 = zippy::spawn(expensive_operation_1);
    let job2 = zippy::spawn(expensive_operation_2);
    let result = job1.wait() + job2.wait();
}
```

Zippy sports a number of features:

- job backlog
- recursive job detection
- panic handling

## Job backlog

If you submit more jobs than you have threads, new jobs will be
put in a backlog which workers will grab from when they are finished.

In the current version, zippy uses X number of threads,
where X is equal to the amount of CPU
cores on the computer running the program.

## Recursive job detection

If a job is recursive (i.e a job that spawns more jobs),
these new jobs get flagged as recursive.
If there are no workers available when a recursive job is submitted using `spawn_rec`
a new thread (refered to as rescue threads in the documentation)
will be spawned to handle this job, instead of being put
in the backlog.
This is done to avoid deadlocks where all workers are waiting on some other
worker to clean up the backlog.

You can also use `spawn` instead, as to minimize the need for creating new threads.
If there are no workers available when `spawn` is called it will
simply execute the job on the current thread.
Rescue threads can still be spawned when using `spawn` but they
will be rarer when compared to using `spawn_rec`.

```rs
// A very expensive version of Fibonacci
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
```

Spawning new threads like this is expensive, and recursive jobs should
ideally be avoided.

## Panic handling

If a job panics zippy automatically handles this,
and a new worker thread will be spawned in its place.
Note that the job that caused the crash is not automatically repeated.
See `Job::try_wait` or `Job::check` for details.
