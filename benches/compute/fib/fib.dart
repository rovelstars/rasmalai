// fib(35): naive double recursion. Prints "RESULT checksum <n>".
int fib(int n) {
  if (n < 2) return n;
  return fib(n - 1) + fib(n - 2);
}

void main() {
  print('RESULT checksum ${fib(35)}');
}
