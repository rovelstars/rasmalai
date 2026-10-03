// spectral-norm N=300, 10 power iterations on AtA.
const N = 300;
function aElem(i, j) {
    // Integer division like every other language: (i+j)*(i+j+1) is always
    // even, so the truncation below is exact for N=300 magnitudes.
    const w = (((i + j) * (i + j + 1)) / 2 | 0) + i + 1;
    return 1.0 / w;
}
function av(x, y) {
    for (let i = 0; i < N; i++) {
        let s = 0.0;
        for (let j = 0; j < N; j++) s += aElem(i, j) * x[j];
        y[i] = s;
    }
}
function atv(x, y) {
    for (let i = 0; i < N; i++) {
        let s = 0.0;
        for (let j = 0; j < N; j++) s += aElem(j, i) * x[j];
        y[i] = s;
    }
}
const u = new Array(N).fill(1.0), v = new Array(N).fill(0.0), tmp = new Array(N).fill(0.0);
for (let k = 0; k < 10; k++) { av(u, tmp); atv(tmp, v); av(v, tmp); atv(tmp, u); }
let vBv = 0.0, vv = 0.0;
for (let j = 0; j < N; j++) { vBv += u[j] * v[j]; vv += v[j] * v[j]; }
console.log('RESULT checksum', Math.sqrt(vBv / vv));
