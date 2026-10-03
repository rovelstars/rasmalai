// matmul 256x256 integer matrices, ijk loop order.
public class Matmul {
    static final int N = 256;

    public static void main(String[] args) {
        long[] a = new long[N * N], b = new long[N * N], c = new long[N * N];
        for (int r = 0; r < N; r++)
            for (int k = 0; k < N; k++) {
                a[r * N + k] = (r + k) % 64;
                b[r * N + k] = (r * k) % 64;
            }
        for (int x = 0; x < N; x++)
            for (int y = 0; y < N; y++) {
                long s = 0;
                for (int z = 0; z < N; z++) s += a[x * N + z] * b[z * N + y];
                c[x * N + y] = s;
            }
        long total = 0;
        for (long v : c) total += v;
        System.out.println("RESULT checksum " + total);
    }
}
