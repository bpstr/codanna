#![allow(dead_code)]

fn identity<T>(value: T) -> T { value }

struct Worker;
impl Worker {
    fn accept<T>(&self, _value: T) {}
}

fn main() {
    let _ = identity::<u32>(1);
    let worker = Worker {};
    worker.accept::<u32>(1);
}
