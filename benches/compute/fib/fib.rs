// fib(35): naive double recursion. Prints "RESULT checksum <n>".
fn fib(n: i64) -> i64 {
    if n < 2 {
        return n;
    }
    fib(n - 1) + fib(n - 2)
}

fn main() {
    println!("RESULT checksum {}", fib(35));
}
