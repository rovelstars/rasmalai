// spectral-norm N=300, 10 power iterations on AtA.
public class Spectral {
    static final int N = 300;

    static double aElem(int i, int j) {
        long w = (long) (i + j) * (i + j + 1) / 2 + i + 1;
        return 1.0 / w;
    }

    static void av(double[] x, double[] y) {
        for (int i = 0; i < N; i++) {
            double s = 0.0;
            for (int j = 0; j < N; j++) s += aElem(i, j) * x[j];
            y[i] = s;
        }
    }

    static void atv(double[] x, double[] y) {
        for (int i = 0; i < N; i++) {
            double s = 0.0;
            for (int j = 0; j < N; j++) s += aElem(j, i) * x[j];
            y[i] = s;
        }
    }

    public static void main(String[] args) {
        double[] u = new double[N], v = new double[N], tmp = new double[N];
        for (int i = 0; i < N; i++) u[i] = 1.0;
        for (int k = 0; k < 10; k++) { av(u, tmp); atv(tmp, v); av(v, tmp); atv(tmp, u); }
        double vBv = 0.0, vv = 0.0;
        for (int j = 0; j < N; j++) { vBv += u[j] * v[j]; vv += v[j] * v[j]; }
        System.out.println("RESULT checksum " + Math.sqrt(vBv / vv));
    }
}
