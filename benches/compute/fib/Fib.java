// fib(35): naive double recursion. Prints "RESULT checksum <n>".
public class Fib {
    static long fib(long n) {
        if (n < 2) return n;
        return fib(n - 1) + fib(n - 2);
    }

    public static void main(String[] args) {
        System.out.println("RESULT checksum " + fib(35));
    }
}
